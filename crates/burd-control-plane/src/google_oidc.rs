use crate::config::GoogleOidcConfig;
use crate::human_auth::VerifiedGoogleIdentity;
use chrono::{DateTime, Utc};
use cookie::{Cookie, CookieJar, Key, SameSite, time::Duration as CookieDuration};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::reqwest::async_http_client;
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use serde::{Deserialize, Serialize};

pub const OIDC_TRANSACTION_COOKIE: &str = "__Host-burd_oidc_tx";
pub const HUMAN_SESSION_COOKIE: &str = "__Host-burd_session";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";

#[derive(Clone, Deserialize)]
pub struct GoogleCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct OidcTransaction {
    state: String,
    nonce: String,
    pkce_verifier: String,
    issued_at: String,
}

pub async fn start_google_oidc(config: &GoogleOidcConfig) -> Result<(String, String), String> {
    let client = discover_client(config).await?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();
    let transaction = OidcTransaction {
        state: state.secret().clone(),
        nonce: nonce.secret().clone(),
        pkce_verifier: pkce_verifier.secret().clone(),
        issued_at: Utc::now().to_rfc3339(),
    };
    let payload = serde_json::to_string(&transaction)
        .map_err(|_| "OIDC transaction could not be created".to_string())?;
    Ok((
        url.to_string(),
        encrypted_transaction_cookie(config, payload)?,
    ))
}

pub async fn complete_google_oidc(
    config: &GoogleOidcConfig,
    cookie_header: Option<&str>,
    query: &GoogleCallbackQuery,
) -> Result<VerifiedGoogleIdentity, String> {
    if query.error.is_some() {
        return Err("OIDC authorization was not completed".to_string());
    }
    let code = query
        .code
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "OIDC callback is invalid".to_string())?;
    let state = query
        .state
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "OIDC callback is invalid".to_string())?;
    let transaction = read_transaction_cookie(config, cookie_header)?;
    let issued_at = DateTime::parse_from_rfc3339(&transaction.issued_at)
        .map_err(|_| "OIDC transaction is invalid".to_string())?
        .with_timezone(&Utc);
    if issued_at > Utc::now()
        || Utc::now().signed_duration_since(issued_at).num_seconds()
            > i64::from(config.transaction_ttl_seconds)
    {
        return Err("OIDC transaction expired".to_string());
    }
    if !constant_time_eq(state.as_bytes(), transaction.state.as_bytes()) {
        return Err("OIDC callback is invalid".to_string());
    }
    let client = discover_client(config).await?;
    let response = client
        .exchange_code(AuthorizationCode::new(code.to_string()))
        .set_pkce_verifier(PkceCodeVerifier::new(transaction.pkce_verifier))
        .request_async(async_http_client)
        .await
        .map_err(|_| "OIDC token exchange failed".to_string())?;
    let id_token = response
        .id_token()
        .ok_or_else(|| "OIDC identity token is missing".to_string())?;
    let claims = id_token
        .claims(&client.id_token_verifier(), &Nonce::new(transaction.nonce))
        .map_err(|_| "OIDC identity token is invalid".to_string())?;
    let subject = claims.subject().as_str().to_string();
    if subject.is_empty() {
        return Err("OIDC identity token is invalid".to_string());
    }
    let email = claims.email().map(|value| value.as_str().to_string());
    let email_verified = claims.email_verified().unwrap_or(false);
    if email.is_some() && !email_verified {
        return Err("OIDC identity token is invalid".to_string());
    }
    Ok(VerifiedGoogleIdentity {
        subject,
        email,
        email_verified,
    })
}

async fn discover_client(config: &GoogleOidcConfig) -> Result<CoreClient, String> {
    #[cfg(test)]
    let issuer = config.test_issuer_url.as_deref().unwrap_or(GOOGLE_ISSUER);
    #[cfg(not(test))]
    let issuer = GOOGLE_ISSUER;
    let metadata = CoreProviderMetadata::discover_async(
        IssuerUrl::new(issuer.to_string())
            .map_err(|_| "Google OIDC configuration is invalid".to_string())?,
        async_http_client,
    )
    .await
    .map_err(|_| "Google OIDC provider is unavailable".to_string())?;
    Ok(CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(config.client_id.clone()),
        Some(ClientSecret::new(config.client_secret.clone())),
    )
    .set_redirect_uri(
        RedirectUrl::new(config.redirect_uri.clone())
            .map_err(|_| "Google OIDC configuration is invalid".to_string())?,
    ))
}

fn encrypted_transaction_cookie(
    config: &GoogleOidcConfig,
    payload: String,
) -> Result<String, String> {
    let key = Key::from(&config.cookie_key.0);
    let mut jar = CookieJar::new();
    let cookie = Cookie::build((OIDC_TRANSACTION_COOKIE, payload))
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(CookieDuration::seconds(i64::from(
            config.transaction_ttl_seconds,
        )))
        .build();
    jar.private_mut(&key).add(cookie);
    jar.delta()
        .next()
        .map(ToString::to_string)
        .ok_or_else(|| "OIDC transaction could not be created".to_string())
}

fn read_transaction_cookie(
    config: &GoogleOidcConfig,
    header: Option<&str>,
) -> Result<OidcTransaction, String> {
    let raw = cookie_value(header, OIDC_TRANSACTION_COOKIE)
        .ok_or_else(|| "OIDC transaction is missing".to_string())?;
    let key = Key::from(&config.cookie_key.0);
    let mut jar = CookieJar::new();
    jar.add_original(Cookie::new(OIDC_TRANSACTION_COOKIE, raw));
    let payload = jar
        .private(&key)
        .get(OIDC_TRANSACTION_COOKIE)
        .ok_or_else(|| "OIDC transaction is invalid".to_string())?
        .value()
        .to_string();
    serde_json::from_str(&payload).map_err(|_| "OIDC transaction is invalid".to_string())
}

pub fn human_session_cookie(token: String, ttl_seconds: u32) -> String {
    Cookie::build((HUMAN_SESSION_COOKIE, token))
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(CookieDuration::seconds(i64::from(ttl_seconds)))
        .build()
        .to_string()
}

pub fn clear_cookie(name: &'static str) -> String {
    Cookie::build((name, ""))
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(CookieDuration::ZERO)
        .build()
        .to_string()
}

pub fn cookie_value(header: Option<&str>, name: &str) -> Option<String> {
    header?
        .split(';')
        .filter_map(|part| Cookie::parse(part.trim().to_string()).ok())
        .find(|cookie| cookie.name() == name)
        .map(|cookie| cookie.value().to_string())
}

#[cfg(test)]
pub(crate) mod oidc_test_support {
    use super::*;
    use axum::extract::{Form, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use base64::Engine as _;
    use openidconnect::core::{
        CoreIdToken, CoreIdTokenClaims, CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey,
    };
    use openidconnect::{
        Audience, EmptyAdditionalClaims, EndUserEmail, JsonWebKeyId, PrivateSigningKey,
        StandardClaims, SubjectIdentifier,
    };
    use rand::rngs::OsRng;
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TokenScenario {
        Valid,
        InvalidPkce,
        WrongNonce,
        WrongIssuer,
        WrongAudience,
        UnknownSigningKey,
        Expired,
        MissingSubject,
        EmailUnverified,
    }

    #[derive(Clone)]
    struct AuthorizationGrant {
        nonce: String,
        pkce_challenge: String,
        scenario: TokenScenario,
    }

    struct MockState {
        issuer: String,
        client_id: String,
        signing_key: CoreRsaPrivateSigningKey,
        unknown_key: CoreRsaPrivateSigningKey,
        jwks: serde_json::Value,
        grants: Mutex<HashMap<String, AuthorizationGrant>>,
        used_codes: Mutex<HashSet<String>>,
    }

    #[derive(Clone)]
    pub struct MockOidcIssuer {
        pub issuer_url: String,
        state: Arc<MockState>,
    }

    #[derive(Clone)]
    pub struct IssuedAuthorizationCode {
        pub code: String,
        pub state: String,
    }

    impl MockOidcIssuer {
        pub async fn start(client_id: &str) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let issuer = format!("http://{}", listener.local_addr().unwrap());
            let signing_key = test_signing_key("known-key");
            let unknown_key = test_signing_key("unknown-key");
            let jwks = serde_json::to_value(openidconnect::core::CoreJsonWebKeySet::new(vec![
                signing_key.as_verification_key(),
            ]))
            .unwrap();
            let state = Arc::new(MockState {
                issuer: issuer.clone(),
                client_id: client_id.to_string(),
                signing_key,
                unknown_key,
                jwks,
                grants: Mutex::new(HashMap::new()),
                used_codes: Mutex::new(HashSet::new()),
            });
            let app = Router::new()
                .route("/.well-known/openid-configuration", get(discovery))
                .route("/authorize", get(authorization_endpoint))
                .route("/token", post(token_endpoint))
                .route("/jwks", get(jwks_endpoint))
                .with_state(state.clone());
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            Self {
                issuer_url: issuer,
                state,
            }
        }

        pub fn issue_code(
            &self,
            authorization_url: &str,
            scenario: TokenScenario,
        ) -> IssuedAuthorizationCode {
            let url = url::Url::parse(authorization_url).unwrap();
            assert_eq!(url.origin().ascii_serialization(), self.issuer_url);
            assert_eq!(url.path(), "/authorize");
            let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
            assert_eq!(query.get("client_id"), Some(&self.state.client_id));
            assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
            assert_eq!(
                query.get("code_challenge_method").map(String::as_str),
                Some("S256")
            );
            let code = format!("mock_code_{}", uuid::Uuid::new_v4());
            let mut challenge = query.get("code_challenge").unwrap().clone();
            if scenario == TokenScenario::InvalidPkce {
                challenge = "invalid-pkce-challenge".to_string();
            }
            self.state.grants.lock().unwrap().insert(
                code.clone(),
                AuthorizationGrant {
                    nonce: query.get("nonce").unwrap().clone(),
                    pkce_challenge: challenge,
                    scenario,
                },
            );
            IssuedAuthorizationCode {
                code,
                state: query.get("state").unwrap().clone(),
            }
        }
    }

    async fn discovery(State(state): State<Arc<MockState>>) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "issuer": state.issuer,
            "authorization_endpoint": format!("{}/authorize", state.issuer),
            "token_endpoint": format!("{}/token", state.issuer),
            "jwks_uri": format!("{}/jwks", state.issuer),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "scopes_supported": ["openid", "email"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"]
        }))
    }

    async fn authorization_endpoint() -> StatusCode {
        StatusCode::NO_CONTENT
    }

    async fn jwks_endpoint(State(state): State<Arc<MockState>>) -> Json<serde_json::Value> {
        Json(state.jwks.clone())
    }

    #[derive(Deserialize)]
    struct TokenForm {
        code: String,
        code_verifier: String,
        grant_type: String,
    }

    async fn token_endpoint(
        State(state): State<Arc<MockState>>,
        Form(form): Form<TokenForm>,
    ) -> impl IntoResponse {
        if form.grant_type != "authorization_code" {
            return oauth_error("unsupported_grant_type");
        }
        let grant = match state.grants.lock().unwrap().get(&form.code).cloned() {
            Some(grant) => grant,
            None => return oauth_error("invalid_grant"),
        };
        if !state.used_codes.lock().unwrap().insert(form.code.clone()) {
            return oauth_error("invalid_grant");
        }
        let actual_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(form.code_verifier.as_bytes()));
        if actual_challenge != grant.pkce_challenge {
            return oauth_error("invalid_grant");
        }
        let token = signed_token(&state, &grant);
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "access_token": "mock-access-token-not-for-persistence",
                "refresh_token": "mock-refresh-token-not-for-persistence",
                "token_type": "Bearer",
                "expires_in": 300,
                "id_token": token.to_string()
            })),
        )
            .into_response()
    }

    fn oauth_error(error: &str) -> axum::response::Response {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response()
    }

    fn signed_token(state: &MockState, grant: &AuthorizationGrant) -> CoreIdToken {
        let issuer = if grant.scenario == TokenScenario::WrongIssuer {
            "https://wrong-issuer.example"
        } else {
            &state.issuer
        };
        let audience = if grant.scenario == TokenScenario::WrongAudience {
            "wrong-client"
        } else {
            &state.client_id
        };
        let nonce = if grant.scenario == TokenScenario::WrongNonce {
            "wrong-nonce"
        } else {
            &grant.nonce
        };
        let subject = if grant.scenario == TokenScenario::MissingSubject {
            ""
        } else {
            "google-sub-new"
        };
        let expires = if grant.scenario == TokenScenario::Expired {
            Utc::now() - chrono::Duration::minutes(1)
        } else {
            Utc::now() + chrono::Duration::minutes(5)
        };
        let claims = CoreIdTokenClaims::new(
            IssuerUrl::new(issuer.to_string()).unwrap(),
            vec![Audience::new(audience.to_string())],
            expires,
            Utc::now(),
            StandardClaims::new(SubjectIdentifier::new(subject.to_string()))
                .set_email(Some(EndUserEmail::new("alice@example.com".to_string())))
                .set_email_verified(Some(grant.scenario != TokenScenario::EmailUnverified)),
            EmptyAdditionalClaims {},
        )
        .set_nonce(Some(Nonce::new(nonce.to_string())));
        let key = if grant.scenario == TokenScenario::UnknownSigningKey {
            &state.unknown_key
        } else {
            &state.signing_key
        };
        CoreIdToken::new(
            claims,
            key,
            CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            None,
            None,
        )
        .unwrap()
    }

    fn test_signing_key(id: &str) -> CoreRsaPrivateSigningKey {
        let key = rsa::RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let pem = key.to_pkcs1_pem(Default::default()).unwrap();
        CoreRsaPrivateSigningKey::from_pem(pem.as_str(), Some(JsonWebKeyId::new(id.to_string())))
            .unwrap()
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SecretCookieKey;
    use openidconnect::core::{
        CoreIdToken, CoreIdTokenClaims, CoreIdTokenVerifier, CoreJsonWebKeySet,
        CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey,
    };
    use openidconnect::{
        Audience, EmptyAdditionalClaims, EndUserEmail, JsonWebKeyId, PrivateSigningKey,
        StandardClaims, SubjectIdentifier,
    };
    use rand::rngs::OsRng;
    use rsa::pkcs1::EncodeRsaPrivateKey;

    fn config() -> GoogleOidcConfig {
        GoogleOidcConfig {
            client_id: "client".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://control.example/v1/auth/google/callback".into(),
            success_url: "https://app.example/auth/success".into(),
            allowed_origins: vec!["https://app.example".into()],
            cookie_key: SecretCookieKey([7; 64]),
            human_session_ttl_seconds: 604800,
            transaction_ttl_seconds: 600,
            test_issuer_url: None,
        }
    }

    #[test]
    fn transaction_cookie_is_private_and_has_browser_guards() {
        let cookie = encrypted_transaction_cookie(&config(), "secret verifier".into()).unwrap();
        assert!(cookie.starts_with("__Host-burd_oidc_tx="));
        assert!(!cookie.contains("secret verifier"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
    }

    #[test]
    fn human_cookie_is_host_only_and_not_readable_by_javascript() {
        let cookie = human_session_cookie("token".into(), 60);
        assert!(cookie.starts_with("__Host-burd_session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(!cookie.contains("Domain="));
    }

    fn transaction_cookie(config: &GoogleOidcConfig, state: &str, issued_at: String) -> String {
        let payload = serde_json::to_string(&OidcTransaction {
            state: state.to_string(),
            nonce: "nonce".to_string(),
            pkce_verifier: "verifier".to_string(),
            issued_at,
        })
        .unwrap();
        encrypted_transaction_cookie(config, payload)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn callback_rejects_missing_transaction_before_network() {
        let query = GoogleCallbackQuery {
            code: Some("code".into()),
            state: Some("state".into()),
            error: None,
        };
        assert!(
            complete_google_oidc(&config(), None, &query)
                .await
                .unwrap_err()
                .contains("missing")
        );
    }

    #[tokio::test]
    async fn callback_rejects_state_mismatch_before_network() {
        let cfg = config();
        let cookie = transaction_cookie(&cfg, "expected", Utc::now().to_rfc3339());
        let query = GoogleCallbackQuery {
            code: Some("code".into()),
            state: Some("other".into()),
            error: None,
        };
        assert!(
            complete_google_oidc(&cfg, Some(&cookie), &query)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn callback_rejects_expired_transaction_before_network() {
        let cfg = config();
        let cookie = transaction_cookie(
            &cfg,
            "state",
            (Utc::now() - chrono::Duration::minutes(11)).to_rfc3339(),
        );
        let query = GoogleCallbackQuery {
            code: Some("code".into()),
            state: Some("state".into()),
            error: None,
        };
        assert!(
            complete_google_oidc(&cfg, Some(&cookie), &query)
                .await
                .unwrap_err()
                .contains("expired")
        );
    }

    fn signing_key(id: &str) -> CoreRsaPrivateSigningKey {
        let key = rsa::RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let pem = key.to_pkcs1_pem(Default::default()).unwrap();
        CoreRsaPrivateSigningKey::from_pem(pem.as_str(), Some(JsonWebKeyId::new(id.to_string())))
            .unwrap()
    }

    fn signed_id_token(
        key: &CoreRsaPrivateSigningKey,
        issuer: &str,
        audience: &str,
        expires: DateTime<Utc>,
        nonce: &str,
    ) -> CoreIdToken {
        let claims = CoreIdTokenClaims::new(
            IssuerUrl::new(issuer.to_string()).unwrap(),
            vec![Audience::new(audience.to_string())],
            expires,
            Utc::now(),
            StandardClaims::new(SubjectIdentifier::new("google-subject".into()))
                .set_email(Some(EndUserEmail::new("user@example.test".into())))
                .set_email_verified(Some(true)),
            EmptyAdditionalClaims {},
        )
        .set_nonce(Some(Nonce::new(nonce.to_string())));
        CoreIdToken::new(
            claims,
            key,
            CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn oidc_library_verifier_rejects_issuer_audience_expiry_nonce_and_signature() {
        let key = signing_key("key-1");
        let verifier = CoreIdTokenVerifier::new_public_client(
            ClientId::new("client".into()),
            IssuerUrl::new(GOOGLE_ISSUER.into()).unwrap(),
            CoreJsonWebKeySet::new(vec![key.as_verification_key()]),
        );
        let nonce = Nonce::new("expected-nonce".into());
        let future = || Utc::now() + chrono::Duration::minutes(5);
        assert!(
            signed_id_token(&key, GOOGLE_ISSUER, "client", future(), "expected-nonce")
                .claims(&verifier, &nonce)
                .is_ok()
        );
        assert!(
            signed_id_token(
                &key,
                "https://wrong.example",
                "client",
                future(),
                "expected-nonce"
            )
            .claims(&verifier, &nonce)
            .is_err()
        );
        assert!(
            signed_id_token(
                &key,
                GOOGLE_ISSUER,
                "wrong-client",
                future(),
                "expected-nonce"
            )
            .claims(&verifier, &nonce)
            .is_err()
        );
        assert!(
            signed_id_token(
                &key,
                GOOGLE_ISSUER,
                "client",
                Utc::now() - chrono::Duration::minutes(1),
                "expected-nonce"
            )
            .claims(&verifier, &nonce)
            .is_err()
        );
        assert!(
            signed_id_token(&key, GOOGLE_ISSUER, "client", future(), "wrong-nonce")
                .claims(&verifier, &nonce)
                .is_err()
        );
        let other = signing_key("key-2");
        assert!(
            signed_id_token(&other, GOOGLE_ISSUER, "client", future(), "expected-nonce")
                .claims(&verifier, &nonce)
                .is_err()
        );
    }
}
