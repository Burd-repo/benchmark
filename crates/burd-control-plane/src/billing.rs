use crate::customer::CustomerApiKeyAuth;
use crate::db::{Database, DbError, IdempotencyRecord, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use burd_protocol::{
    BILLING_INVOICE_SCHEMA_VERSION, BillingBalance, BillingBalanceResponse, BillingInvoiceRecord,
    BillingInvoiceResponse, ConfirmPixPaymentIntentRequest, CreatePixPaymentIntentRequest,
    CreateProviderPayoutRequest, FINANCIAL_LEDGER_SCHEMA_VERSION, FinancialLedgerLineRecord,
    FinancialLedgerResponse, MARKETPLACE_PRICE_SCHEMA_VERSION, MarketplacePriceRecord,
    MarketplacePriceResponse, PIX_PAYMENT_INTENT_SCHEMA_VERSION,
    PROVIDER_PAYOUT_ACCOUNT_SCHEMA_VERSION, PROVIDER_PAYOUT_SCHEMA_VERSION, PixPaymentIntentRecord,
    PixPaymentIntentResponse, ProviderPayoutAccountRecord, ProviderPayoutAccountResponse,
    ProviderPayoutRecord, ProviderPayoutResponse, SettleReservationBillingRequest,
    UpsertMarketplacePriceRequest, UpsertProviderPayoutAccountRequest, hash_canonical,
};
use chrono::{Duration, Utc};
use tokio_postgres::{GenericClient, Row, Transaction};
use uuid::Uuid;

const DEFAULT_PLATFORM_FEE_BPS: u32 = 1500;
const DEFAULT_CHARGEBACK_RESERVE_BPS: u32 = 500;
const DEFAULT_MINIMUM_PAYOUT_MICROS: u64 = 5_000_000;
const DEFAULT_PAYOUT_HOLD_DAYS: u32 = 7;
const MAX_FINANCIAL_LEDGER_LIMIT: u32 = 200;
const PIX_ADAPTER_PROVIDER: &str = "pix_manual_adapter_v1";

#[derive(Debug, Clone)]
pub struct CreatePixPaymentIntentCommand {
    pub request_id: String,
    pub scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub auth: CustomerApiKeyAuth,
    pub project_id: String,
    pub request: CreatePixPaymentIntentRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreatePixPaymentIntentOutcome {
    Response(IdempotencyRecord),
    Conflict,
}

#[derive(Debug, Clone)]
struct ReservationBillingSource {
    reservation_id: String,
    organization_id: String,
    project_id: String,
    listing_id: String,
    provider_id: String,
    device_id: String,
    gpu_uuid: Option<String>,
    status: String,
}

#[derive(Debug, Clone)]
struct UsageBillingSource {
    entry_id: String,
    provider_id: String,
    device_id: String,
    gpu_uuid: String,
    job_id: String,
    billable_gpu_seconds: u64,
    receipt_hash: String,
    source_hash: String,
}

#[derive(Debug, Clone)]
struct ActivePrice {
    price_id: String,
    currency: String,
    price_per_hour_micros: u64,
}

#[derive(Debug, Clone)]
struct LedgerLine {
    account_type: &'static str,
    owner_type: &'static str,
    owner_id: Option<String>,
    amount_micros: i64,
    description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SettlementAmounts {
    subtotal_micros: u64,
    platform_fee_micros: u64,
    provider_net_micros: u64,
    chargeback_reserve_micros: u64,
}

impl Database {
    pub async fn upsert_marketplace_price(
        &self,
        request_id: &str,
        listing_id: &str,
        request: &UpsertMarketplacePriceRequest,
    ) -> Result<MarketplacePriceResponse, SessionError> {
        validate_id("listing_id", listing_id, 160)?;
        validate_price_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        let listing_exists = transaction
            .query_opt(
                "SELECT listing_id FROM marketplace_listings WHERE listing_id = $1 FOR UPDATE",
                &[&listing_id],
            )
            .await?
            .is_some();
        if !listing_exists {
            return Err(SessionError::NotFound(
                "marketplace listing not found".to_string(),
            ));
        }
        transaction
            .execute(
                "UPDATE marketplace_listing_prices SET status = 'superseded', updated_at = $1 WHERE listing_id = $2 AND status = 'active'",
                &[&now, &listing_id],
            )
            .await?;
        let price_id = format!("price_{}", Uuid::new_v4());
        let pricing_model = request
            .pricing_model
            .clone()
            .unwrap_or_else(|| "gpu_hour".to_string());
        transaction
            .execute(
                "INSERT INTO marketplace_listing_prices (price_id, listing_id, schema_version, currency, price_per_hour_micros, pricing_model, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, 'active', $7, $7)",
                &[
                    &price_id,
                    &listing_id,
                    &MARKETPLACE_PRICE_SCHEMA_VERSION,
                    &request.currency,
                    &to_i64(request.price_per_hour_micros)?,
                    &pricing_model,
                    &now,
                ],
            )
            .await?;
        transaction
            .execute(
                "UPDATE marketplace_listings SET price_currency = $1, price_per_hour_micros = $2, price_source = 'billing_price_book', updated_at = $3 WHERE listing_id = $4",
                &[
                    &request.currency,
                    &to_i64(request.price_per_hour_micros)?,
                    &now,
                    &listing_id,
                ],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "marketplace_listing_price",
                entity_id: &price_id,
                event_type: "billing.price.updated",
                idempotency_key: None,
                summary: "marketplace listing price configured for billing",
                metadata_json: &serde_json::json!({ "listing_id": listing_id }).to_string(),
            },
        )
        .await?;
        let price = load_price(&transaction, &price_id).await?;
        transaction.commit().await?;
        Ok(MarketplacePriceResponse {
            request_id: request_id.to_string(),
            price,
        })
    }

    pub async fn create_pix_payment_intent_idempotently(
        &self,
        command: CreatePixPaymentIntentCommand,
    ) -> Result<CreatePixPaymentIntentOutcome, SessionError> {
        require_customer_scope(&command.auth, "billing:write")?;
        validate_id("project_id", &command.project_id, 128)?;
        validate_pix_intent_request(&command.request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let organization_id =
            authorize_project_access(&transaction, &command.auth, &command.project_id).await?;
        let now = Utc::now().to_rfc3339();
        let reserved = transaction
            .execute(
                "INSERT INTO idempotency_keys (scope, idempotency_key, request_hash, status_code, response_json, created_at) VALUES ($1, $2, $3, 0, '', $4) ON CONFLICT (scope, idempotency_key) DO NOTHING",
                &[&command.scope, &command.idempotency_key, &command.request_hash, &now],
            )
            .await?
            == 1;
        if !reserved {
            let row = transaction
                .query_one(
                    "SELECT request_hash, status_code, response_json FROM idempotency_keys WHERE scope = $1 AND idempotency_key = $2 FOR UPDATE",
                    &[&command.scope, &command.idempotency_key],
                )
                .await?;
            let record = idempotency_from_row(row);
            transaction.commit().await?;
            return if record.request_hash == command.request_hash {
                Ok(CreatePixPaymentIntentOutcome::Response(record))
            } else {
                Ok(CreatePixPaymentIntentOutcome::Conflict)
            };
        }

        let payment_intent_id = format!("pix_{}", Uuid::new_v4());
        transaction
            .execute(
                "INSERT INTO pix_payment_intents (payment_intent_id, organization_id, project_id, schema_version, status, amount_micros, currency, provider, external_reference, adapter_status, idempotency_key, request_hash, created_at, updated_at) VALUES ($1, $2, $3, $4, 'requires_confirmation', $5, $6, $7, $8, 'adapter_not_invoked_manual_confirmation_required', $9, $10, $11, $11)",
                &[
                    &payment_intent_id,
                    &organization_id,
                    &command.project_id,
                    &PIX_PAYMENT_INTENT_SCHEMA_VERSION,
                    &to_i64(command.request.amount_micros)?,
                    &command.request.currency,
                    &PIX_ADAPTER_PROVIDER,
                    &command.request.external_reference,
                    &command.idempotency_key,
                    &command.request_hash,
                    &now,
                ],
            )
            .await?;
        insert_customer_audit_event(
            &transaction,
            CustomerAuditEvent {
                organization_id: &organization_id,
                project_id: Some(command.project_id.clone()),
                actor_type: "customer_api_key",
                actor_id: Some(command.auth.api_key_id.clone()),
                event_type: "billing.pix_payment_intent.created",
                entity_type: "pix_payment_intent",
                entity_id: &payment_intent_id,
                summary: "Pix payment intent created without ledger movement",
                metadata_json: serde_json::json!({
                    "amount_micros": command.request.amount_micros,
                    "currency": command.request.currency,
                })
                .to_string(),
            },
        )
        .await?;
        let payment_intent = load_pix_payment_intent(&transaction, &payment_intent_id).await?;
        let response_json = serde_json::to_string(&PixPaymentIntentResponse {
            request_id: command.request_id,
            payment_intent,
            duplicate: false,
        })
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
        let status_code = 201_i32;
        transaction
            .execute(
                "UPDATE idempotency_keys SET status_code = $1, response_json = $2 WHERE scope = $3 AND idempotency_key = $4",
                &[&status_code, &response_json, &command.scope, &command.idempotency_key],
            )
            .await?;
        transaction.commit().await?;
        Ok(CreatePixPaymentIntentOutcome::Response(IdempotencyRecord {
            request_hash: command.request_hash,
            status_code: status_code as u16,
            response_json,
        }))
    }

    pub async fn confirm_pix_payment_intent(
        &self,
        request_id: &str,
        payment_intent_id: &str,
        request: &ConfirmPixPaymentIntentRequest,
    ) -> Result<PixPaymentIntentResponse, SessionError> {
        validate_id("payment_intent_id", payment_intent_id, 160)?;
        validate_confirm_pix_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let before = load_pix_payment_intent_for_update(&transaction, payment_intent_id).await?;
        let duplicate = before.status == "confirmed";
        if !duplicate {
            let confirmed_at = request
                .paid_at
                .clone()
                .unwrap_or_else(|| Utc::now().to_rfc3339());
            append_financial_transaction(
                &transaction,
                "pix_payment_intent",
                payment_intent_id,
                &before.currency,
                vec![
                    LedgerLine {
                        account_type: "customer_balance",
                        owner_type: "project",
                        owner_id: Some(before.project_id.clone()),
                        amount_micros: to_i64(before.amount_micros)?,
                        description: "Pix payment credited to customer balance",
                    },
                    LedgerLine {
                        account_type: "payment_processor_clearing",
                        owner_type: "platform",
                        owner_id: None,
                        amount_micros: -to_i64(before.amount_micros)?,
                        description: "Pix processor clearing offset",
                    },
                ],
            )
            .await?;
            transaction
                .execute(
                    "UPDATE pix_payment_intents SET status = 'confirmed', provider = $1, external_reference = $2, adapter_status = 'confirmed_by_admin_or_webhook', confirmed_at = $3, updated_at = $3 WHERE payment_intent_id = $4",
                    &[&request.provider, &request.external_reference, &confirmed_at, &payment_intent_id],
                )
                .await?;
            insert_customer_audit_event(
                &transaction,
                CustomerAuditEvent {
                    organization_id: &before.organization_id,
                    project_id: Some(before.project_id.clone()),
                    actor_type: "admin",
                    actor_id: None,
                    event_type: "billing.pix_payment_intent.confirmed",
                    entity_type: "pix_payment_intent",
                    entity_id: payment_intent_id,
                    summary: "Pix payment intent confirmed and credited",
                    metadata_json: serde_json::json!({
                        "provider": request.provider,
                        "external_reference": request.external_reference,
                    })
                    .to_string(),
                },
            )
            .await?;
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "admin",
                    actor_id: None,
                    entity_type: "pix_payment_intent",
                    entity_id: payment_intent_id,
                    event_type: "billing.pix.confirmed",
                    idempotency_key: None,
                    summary: "Pix payment confirmed into financial ledger",
                    metadata_json: "{}",
                },
            )
            .await?;
        }
        let payment_intent = load_pix_payment_intent(&transaction, payment_intent_id).await?;
        transaction.commit().await?;
        Ok(PixPaymentIntentResponse {
            request_id: request_id.to_string(),
            payment_intent,
            duplicate,
        })
    }

    pub async fn settle_reservation_billing(
        &self,
        request_id: &str,
        reservation_id: &str,
        request: &SettleReservationBillingRequest,
    ) -> Result<BillingInvoiceResponse, SessionError> {
        validate_id("reservation_id", reservation_id, 160)?;
        validate_id("usage_entry_id", &request.usage_entry_id, 160)?;
        let platform_fee_bps = request.platform_fee_bps.unwrap_or(DEFAULT_PLATFORM_FEE_BPS);
        let reserve_bps = request
            .chargeback_reserve_bps
            .unwrap_or(DEFAULT_CHARGEBACK_RESERVE_BPS);
        validate_bps(platform_fee_bps, reserve_bps)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let reservation = load_reservation_billing_source(&transaction, reservation_id).await?;
        if reservation.status == "cancelled" {
            return Err(SessionError::Conflict(
                "cancelled reservations cannot be billed".to_string(),
            ));
        }
        let usage = load_usage_billing_source(&transaction, &request.usage_entry_id).await?;
        validate_usage_matches_reservation(&reservation, &usage)?;
        if let Some(row) = transaction
            .query_opt(
                &format!(
                    "{} WHERE reservation_id = $1 AND usage_entry_id = $2",
                    invoice_select_columns()
                ),
                &[&reservation_id, &request.usage_entry_id],
            )
            .await?
        {
            transaction.commit().await?;
            return Ok(BillingInvoiceResponse {
                request_id: request_id.to_string(),
                invoice: invoice_from_row(row)?,
                duplicate: true,
            });
        }
        if usage.billable_gpu_seconds == 0 {
            return Err(SessionError::Conflict(
                "usage entry has no billable GPU seconds".to_string(),
            ));
        }
        let price = load_active_price(&transaction, &reservation.listing_id).await?;
        let subtotal =
            billable_amount_micros(usage.billable_gpu_seconds, price.price_per_hour_micros)?;
        let amounts = settlement_amounts(subtotal, platform_fee_bps, reserve_bps)?;
        let subtotal_i64 = to_i64(amounts.subtotal_micros)?;
        let customer_balance = account_balance(
            &transaction,
            "customer_balance",
            "project",
            &reservation.project_id,
            &price.currency,
        )
        .await?;
        if customer_balance < subtotal_i64 {
            return Err(SessionError::Conflict(
                "project customer balance is insufficient for billing settlement".to_string(),
            ));
        }
        let source_hash = hash_canonical(&serde_json::json!({
            "reservation_id": reservation.reservation_id,
            "usage_entry_id": usage.entry_id,
            "usage_source_hash": usage.source_hash,
            "usage_receipt_hash": usage.receipt_hash,
            "price_id": price.price_id,
            "price_per_hour_micros": price.price_per_hour_micros,
            "billable_gpu_seconds": usage.billable_gpu_seconds,
            "platform_fee_bps": platform_fee_bps,
            "chargeback_reserve_bps": reserve_bps,
        }))
        .map_err(SessionError::Invalid)?;
        let invoice_id = format!("invoice_{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        transaction
            .execute(
                "INSERT INTO billing_invoices (invoice_id, organization_id, project_id, reservation_id, usage_entry_id, schema_version, status, currency, subtotal_micros, platform_fee_micros, provider_net_micros, chargeback_reserve_micros, total_micros, source_hash, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, 'issued', $7, $8, $9, $10, $11, $12, $13, $14, $14)",
                &[
                    &invoice_id,
                    &reservation.organization_id,
                    &reservation.project_id,
                    &Some(reservation_id.to_string()),
                    &Some(request.usage_entry_id.clone()),
                    &BILLING_INVOICE_SCHEMA_VERSION,
                    &price.currency,
                    &subtotal_i64,
                    &to_i64(amounts.platform_fee_micros)?,
                    &to_i64(amounts.provider_net_micros)?,
                    &to_i64(amounts.chargeback_reserve_micros)?,
                    &subtotal_i64,
                    &source_hash,
                    &now,
                ],
            )
            .await?;
        append_financial_transaction(
            &transaction,
            "billing_invoice",
            &invoice_id,
            &price.currency,
            vec![
                LedgerLine {
                    account_type: "customer_balance",
                    owner_type: "project",
                    owner_id: Some(reservation.project_id.clone()),
                    amount_micros: -subtotal_i64,
                    description: "customer billed for metered GPU usage",
                },
                LedgerLine {
                    account_type: "provider_payable",
                    owner_type: "provider",
                    owner_id: Some(reservation.provider_id.clone()),
                    amount_micros: to_i64(amounts.provider_net_micros)?,
                    description: "provider net payable for metered usage",
                },
                LedgerLine {
                    account_type: "platform_revenue",
                    owner_type: "platform",
                    owner_id: None,
                    amount_micros: to_i64(amounts.platform_fee_micros)?,
                    description: "platform fee earned on invoice",
                },
                LedgerLine {
                    account_type: "chargeback_reserve",
                    owner_type: "platform",
                    owner_id: None,
                    amount_micros: to_i64(amounts.chargeback_reserve_micros)?,
                    description: "chargeback reserve withheld from provider payout",
                },
            ],
        )
        .await?;
        insert_customer_audit_event(
            &transaction,
            CustomerAuditEvent {
                organization_id: &reservation.organization_id,
                project_id: Some(reservation.project_id.clone()),
                actor_type: "admin",
                actor_id: None,
                event_type: "billing.invoice.issued",
                entity_type: "billing_invoice",
                entity_id: &invoice_id,
                summary: "reservation usage settled into billing invoice",
                metadata_json: serde_json::json!({
                    "usage_entry_id": usage.entry_id,
                    "job_id": usage.job_id,
                })
                .to_string(),
            },
        )
        .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "billing_invoice",
                entity_id: &invoice_id,
                event_type: "billing.invoice.issued",
                idempotency_key: None,
                summary: "metered reservation usage settled into financial ledger",
                metadata_json: &serde_json::json!({
                    "reservation_id": reservation_id,
                    "usage_entry_id": request.usage_entry_id,
                })
                .to_string(),
            },
        )
        .await?;
        let invoice = load_invoice(&transaction, &invoice_id).await?;
        transaction.commit().await?;
        Ok(BillingInvoiceResponse {
            request_id: request_id.to_string(),
            invoice,
            duplicate: false,
        })
    }
}

impl Database {
    pub async fn get_billing_invoice(
        &self,
        request_id: &str,
        invoice_id: &str,
    ) -> Result<BillingInvoiceResponse, SessionError> {
        validate_id("invoice_id", invoice_id, 160)?;
        let client = self.connect().await?;
        let row = client
            .query_opt(
                &format!("{} WHERE invoice_id = $1", invoice_select_columns()),
                &[&invoice_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("billing invoice not found".to_string()))?;
        Ok(BillingInvoiceResponse {
            request_id: request_id.to_string(),
            invoice: invoice_from_row(row)?,
            duplicate: false,
        })
    }

    pub async fn project_billing_balance(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
    ) -> Result<BillingBalanceResponse, SessionError> {
        require_customer_scope(auth, "billing:read")?;
        validate_id("project_id", project_id, 128)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        authorize_project_access(&transaction, auth, project_id).await?;
        let balances = load_balances(&transaction, "project", project_id).await?;
        transaction.commit().await?;
        Ok(BillingBalanceResponse {
            request_id: request_id.to_string(),
            balances,
        })
    }

    pub async fn provider_billing_balance(
        &self,
        request_id: &str,
        provider_id: &str,
    ) -> Result<BillingBalanceResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let client = self.connect().await?;
        let balances = load_balances(&client, "provider", provider_id).await?;
        Ok(BillingBalanceResponse {
            request_id: request_id.to_string(),
            balances,
        })
    }

    pub async fn list_project_financial_ledger(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        limit: u32,
    ) -> Result<FinancialLedgerResponse, SessionError> {
        require_customer_scope(auth, "billing:read")?;
        validate_id("project_id", project_id, 128)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        authorize_project_access(&transaction, auth, project_id).await?;
        let lines = load_ledger_lines(&transaction, "project", project_id, limit).await?;
        transaction.commit().await?;
        Ok(FinancialLedgerResponse {
            request_id: request_id.to_string(),
            lines,
        })
    }

    pub async fn list_provider_financial_ledger(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<FinancialLedgerResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let client = self.connect().await?;
        let lines = load_ledger_lines(&client, "provider", provider_id, limit).await?;
        Ok(FinancialLedgerResponse {
            request_id: request_id.to_string(),
            lines,
        })
    }

    pub async fn upsert_provider_payout_account(
        &self,
        request_id: &str,
        provider_id: &str,
        request: &UpsertProviderPayoutAccountRequest,
    ) -> Result<ProviderPayoutAccountResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        validate_payout_account_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        if transaction
            .query_opt(
                "SELECT provider_id FROM providers WHERE provider_id = $1",
                &[&provider_id],
            )
            .await?
            .is_none()
        {
            return Err(SessionError::NotFound("provider not found".to_string()));
        }
        let now = Utc::now().to_rfc3339();
        let existing = transaction
            .query_opt(
                "SELECT payout_account_id FROM provider_payout_accounts WHERE provider_id = $1 AND payout_method = $2 AND currency = $3 FOR UPDATE",
                &[&provider_id, &request.payout_method, &request.currency],
            )
            .await?;
        let payout_account_id = existing
            .map(|row| row.get::<_, String>("payout_account_id"))
            .unwrap_or_else(|| format!("payout_acct_{}", Uuid::new_v4()));
        let minimum = request
            .minimum_payout_micros
            .unwrap_or(DEFAULT_MINIMUM_PAYOUT_MICROS);
        let hold_days = request.payout_hold_days.unwrap_or(DEFAULT_PAYOUT_HOLD_DAYS);
        transaction
            .execute(
                "INSERT INTO provider_payout_accounts (payout_account_id, provider_id, schema_version, payout_method, currency, pix_key_hash, pix_key_last4, kyc_status, tax_status, minimum_payout_micros, payout_hold_days, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'active', $12, $12) ON CONFLICT (provider_id, payout_method, currency) DO UPDATE SET pix_key_hash = EXCLUDED.pix_key_hash, pix_key_last4 = EXCLUDED.pix_key_last4, kyc_status = EXCLUDED.kyc_status, tax_status = EXCLUDED.tax_status, minimum_payout_micros = EXCLUDED.minimum_payout_micros, payout_hold_days = EXCLUDED.payout_hold_days, status = 'active', updated_at = EXCLUDED.updated_at",
                &[
                    &payout_account_id,
                    &provider_id,
                    &PROVIDER_PAYOUT_ACCOUNT_SCHEMA_VERSION,
                    &request.payout_method,
                    &request.currency,
                    &request.pix_key_hash,
                    &request.pix_key_last4,
                    &request.kyc_status,
                    &request.tax_status,
                    &to_i64(minimum)?,
                    &(hold_days as i32),
                    &now,
                ],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "provider_payout_account",
                entity_id: &payout_account_id,
                event_type: "billing.payout_account.upserted",
                idempotency_key: None,
                summary: "provider payout account updated",
                metadata_json: "{}",
            },
        )
        .await?;
        let payout_account = load_payout_account(&transaction, &payout_account_id).await?;
        transaction.commit().await?;
        Ok(ProviderPayoutAccountResponse {
            request_id: request_id.to_string(),
            payout_account,
        })
    }

    pub async fn create_provider_payout(
        &self,
        request_id: &str,
        provider_id: &str,
        request: &CreateProviderPayoutRequest,
    ) -> Result<ProviderPayoutResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        validate_money(request.amount_micros, &request.currency)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let account =
            load_active_payout_account(&transaction, provider_id, &request.currency).await?;
        if account.kyc_status != "verified" || account.tax_status != "verified" {
            return Err(SessionError::Conflict(
                "provider KYC and tax status must be verified before payout".to_string(),
            ));
        }
        if request.amount_micros < account.minimum_payout_micros {
            return Err(SessionError::Conflict(
                "payout amount is below provider minimum payout".to_string(),
            ));
        }
        let payable = account_balance(
            &transaction,
            "provider_payable",
            "provider",
            provider_id,
            &request.currency,
        )
        .await?;
        if payable < to_i64(request.amount_micros)? {
            return Err(SessionError::Conflict(
                "provider payable balance is insufficient for payout".to_string(),
            ));
        }
        let payout_id = format!("payout_{}", Uuid::new_v4());
        let now_dt = Utc::now();
        let now = now_dt.to_rfc3339();
        let hold_until = if account.payout_hold_days == 0 {
            None
        } else {
            Some((now_dt + Duration::days(i64::from(account.payout_hold_days))).to_rfc3339())
        };
        let status = if hold_until.is_some() {
            "held"
        } else {
            "approved"
        };
        transaction
            .execute(
                "INSERT INTO provider_payouts (payout_id, provider_id, payout_account_id, schema_version, status, amount_micros, currency, hold_until, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
                &[
                    &payout_id,
                    &provider_id,
                    &account.payout_account_id,
                    &PROVIDER_PAYOUT_SCHEMA_VERSION,
                    &status,
                    &to_i64(request.amount_micros)?,
                    &request.currency,
                    &hold_until,
                    &now,
                ],
            )
            .await?;
        append_financial_transaction(
            &transaction,
            "provider_payout",
            &payout_id,
            &request.currency,
            vec![
                LedgerLine {
                    account_type: "provider_payable",
                    owner_type: "provider",
                    owner_id: Some(provider_id.to_string()),
                    amount_micros: -to_i64(request.amount_micros)?,
                    description: "provider payable reserved for payout",
                },
                LedgerLine {
                    account_type: "provider_payout_clearing",
                    owner_type: "platform",
                    owner_id: None,
                    amount_micros: to_i64(request.amount_micros)?,
                    description: "provider payout clearing liability",
                },
            ],
        )
        .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "provider_payout",
                entity_id: &payout_id,
                event_type: "billing.payout.created",
                idempotency_key: None,
                summary: "provider payout created from payable balance",
                metadata_json: "{}",
            },
        )
        .await?;
        let payout = load_payout(&transaction, &payout_id).await?;
        transaction.commit().await?;
        Ok(ProviderPayoutResponse {
            request_id: request_id.to_string(),
            payout,
        })
    }
}

async fn append_financial_transaction(
    transaction: &Transaction<'_>,
    source_type: &str,
    source_id: &str,
    currency: &str,
    lines: Vec<LedgerLine>,
) -> Result<Vec<FinancialLedgerLineRecord>, SessionError> {
    validate_id("source_type", source_type, 64)?;
    validate_id("source_id", source_id, 160)?;
    validate_currency(currency)?;
    let lines = lines
        .into_iter()
        .filter(|line| line.amount_micros != 0)
        .collect::<Vec<_>>();
    assert_balanced(&lines)?;
    let transaction_id = format!("txn_{}", Uuid::new_v4());
    let now = Utc::now().to_rfc3339();
    let mut records = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        validate_ledger_line(&line)?;
        let ledger_line_id = format!("ledger_{}", Uuid::new_v4());
        let line_number = i32::try_from(index + 1)
            .map_err(|_| SessionError::Invalid("ledger line index overflow".to_string()))?;
        transaction
            .execute(
                "INSERT INTO financial_ledger_lines (ledger_line_id, transaction_id, schema_version, line_number, account_type, account_owner_type, account_owner_id, currency, amount_micros, source_type, source_id, description, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                &[
                    &ledger_line_id,
                    &transaction_id,
                    &FINANCIAL_LEDGER_SCHEMA_VERSION,
                    &line_number,
                    &line.account_type,
                    &line.owner_type,
                    &line.owner_id,
                    &currency,
                    &line.amount_micros,
                    &source_type,
                    &source_id,
                    &line.description,
                    &now,
                ],
            )
            .await?;
        records.push(FinancialLedgerLineRecord {
            ledger_line_id,
            transaction_id: transaction_id.clone(),
            schema_version: FINANCIAL_LEDGER_SCHEMA_VERSION.to_string(),
            line_number: line_number as u32,
            account_type: line.account_type.to_string(),
            account_owner_type: line.owner_type.to_string(),
            account_owner_id: line.owner_id,
            currency: currency.to_string(),
            amount_micros: line.amount_micros,
            source_type: source_type.to_string(),
            source_id: source_id.to_string(),
            description: line.description.to_string(),
            created_at: now.clone(),
        });
    }
    Ok(records)
}

fn assert_balanced(lines: &[LedgerLine]) -> Result<(), SessionError> {
    if lines.len() < 2 {
        return Err(SessionError::Invalid(
            "financial transaction requires at least two non-zero ledger lines".to_string(),
        ));
    }
    let sum: i128 = lines
        .iter()
        .map(|line| i128::from(line.amount_micros))
        .sum();
    if sum == 0 {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "financial ledger transaction must balance to zero".to_string(),
        ))
    }
}

fn validate_ledger_line(line: &LedgerLine) -> Result<(), SessionError> {
    validate_id("account_type", line.account_type, 64)?;
    validate_id("account_owner_type", line.owner_type, 64)?;
    if let Some(owner_id) = line.owner_id.as_deref() {
        validate_id("account_owner_id", owner_id, 160)?;
    }
    if !is_bounded_ascii(line.description, 160) {
        return Err(SessionError::Invalid(
            "ledger line description must be printable ASCII".to_string(),
        ));
    }
    Ok(())
}

async fn authorize_project_access(
    transaction: &Transaction<'_>,
    auth: &CustomerApiKeyAuth,
    project_id: &str,
) -> Result<String, SessionError> {
    if auth
        .project_id
        .as_deref()
        .is_some_and(|bound| bound != project_id)
    {
        return Err(SessionError::Unauthorized);
    }
    let row = transaction
        .query_opt(
            "SELECT p.organization_id, p.status AS project_status, o.status AS organization_status FROM projects p JOIN organizations o ON o.organization_id = p.organization_id WHERE p.project_id = $1",
            &[&project_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("project not found".to_string()))?;
    let organization_id: String = row.get("organization_id");
    let project_status: String = row.get("project_status");
    let organization_status: String = row.get("organization_status");
    if organization_id != auth.organization_id {
        return Err(SessionError::Unauthorized);
    }
    if project_status != "active" || organization_status != "active" {
        return Err(SessionError::Conflict(
            "project or organization is not active".to_string(),
        ));
    }
    Ok(organization_id)
}

struct CustomerAuditEvent<'a> {
    organization_id: &'a str,
    project_id: Option<String>,
    actor_type: &'a str,
    actor_id: Option<String>,
    event_type: &'a str,
    entity_type: &'a str,
    entity_id: &'a str,
    summary: &'a str,
    metadata_json: String,
}

async fn insert_customer_audit_event(
    transaction: &Transaction<'_>,
    event: CustomerAuditEvent<'_>,
) -> Result<(), SessionError> {
    let event_id = format!("caudit_{}", Uuid::new_v4());
    transaction
        .execute(
            "INSERT INTO customer_audit_events (customer_audit_event_id, organization_id, project_id, schema_version, actor_type, actor_id, event_type, entity_type, entity_id, summary, metadata_json, occurred_at) VALUES ($1, $2, $3, 'burd-customer-audit-v1', $4, $5, $6, $7, $8, $9, $10, $11)",
            &[
                &event_id,
                &event.organization_id,
                &event.project_id,
                &event.actor_type,
                &event.actor_id,
                &event.event_type,
                &event.entity_type,
                &event.entity_id,
                &event.summary,
                &event.metadata_json,
                &Utc::now().to_rfc3339(),
            ],
        )
        .await?;
    Ok(())
}

async fn load_price(
    transaction: &Transaction<'_>,
    price_id: &str,
) -> Result<MarketplacePriceRecord, SessionError> {
    let row = transaction
        .query_one(
            &format!("{} WHERE price_id = $1", price_select_columns()),
            &[&price_id],
        )
        .await?;
    price_from_row(row)
}

async fn load_active_price(
    transaction: &Transaction<'_>,
    listing_id: &str,
) -> Result<ActivePrice, SessionError> {
    let row = transaction
        .query_opt(
            "SELECT price_id, currency, price_per_hour_micros FROM marketplace_listing_prices WHERE listing_id = $1 AND status = 'active' ORDER BY updated_at DESC LIMIT 1",
            &[&listing_id],
        )
        .await?
        .ok_or_else(|| SessionError::Conflict("listing has no active billing price".to_string()))?;
    Ok(ActivePrice {
        price_id: row.get("price_id"),
        currency: row.get("currency"),
        price_per_hour_micros: from_i64_to_u64(row.get("price_per_hour_micros"))?,
    })
}

async fn load_pix_payment_intent(
    transaction: &Transaction<'_>,
    payment_intent_id: &str,
) -> Result<PixPaymentIntentRecord, SessionError> {
    let row = transaction
        .query_one(
            &format!("{} WHERE payment_intent_id = $1", pix_select_columns()),
            &[&payment_intent_id],
        )
        .await?;
    pix_from_row(row)
}

async fn load_pix_payment_intent_for_update(
    transaction: &Transaction<'_>,
    payment_intent_id: &str,
) -> Result<PixPaymentIntentRecord, SessionError> {
    let row = transaction
        .query_opt(
            &format!(
                "{} WHERE payment_intent_id = $1 FOR UPDATE",
                pix_select_columns()
            ),
            &[&payment_intent_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("Pix payment intent not found".to_string()))?;
    pix_from_row(row)
}

async fn load_reservation_billing_source(
    transaction: &Transaction<'_>,
    reservation_id: &str,
) -> Result<ReservationBillingSource, SessionError> {
    let row = transaction
        .query_opt(
            "SELECT reservation_id, organization_id, project_id, listing_id, provider_id, device_id, gpu_uuid, status FROM marketplace_reservations WHERE reservation_id = $1 FOR UPDATE",
            &[&reservation_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("reservation not found".to_string()))?;
    Ok(ReservationBillingSource {
        reservation_id: row.get("reservation_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        listing_id: row.get("listing_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        gpu_uuid: row.get("gpu_uuid"),
        status: row.get("status"),
    })
}

async fn load_usage_billing_source(
    transaction: &Transaction<'_>,
    usage_entry_id: &str,
) -> Result<UsageBillingSource, SessionError> {
    let row = transaction
        .query_opt(
            "SELECT entry_id, provider_id, device_id, gpu_uuid, job_id, billable_gpu_seconds, receipt_hash, source_hash FROM usage_ledger_entries WHERE entry_id = $1 FOR UPDATE",
            &[&usage_entry_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("usage ledger entry not found".to_string()))?;
    Ok(UsageBillingSource {
        entry_id: row.get("entry_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        gpu_uuid: row.get("gpu_uuid"),
        job_id: row.get("job_id"),
        billable_gpu_seconds: from_i64_to_u64(row.get("billable_gpu_seconds"))?,
        receipt_hash: row.get("receipt_hash"),
        source_hash: row.get("source_hash"),
    })
}

fn validate_usage_matches_reservation(
    reservation: &ReservationBillingSource,
    usage: &UsageBillingSource,
) -> Result<(), SessionError> {
    if reservation.provider_id != usage.provider_id || reservation.device_id != usage.device_id {
        return Err(SessionError::Conflict(
            "usage entry does not match reservation provider/device".to_string(),
        ));
    }
    if reservation
        .gpu_uuid
        .as_deref()
        .is_some_and(|gpu_uuid| gpu_uuid != usage.gpu_uuid)
    {
        return Err(SessionError::Conflict(
            "usage entry does not match reservation GPU UUID".to_string(),
        ));
    }
    Ok(())
}

async fn load_invoice(
    transaction: &Transaction<'_>,
    invoice_id: &str,
) -> Result<BillingInvoiceRecord, SessionError> {
    let row = transaction
        .query_one(
            &format!("{} WHERE invoice_id = $1", invoice_select_columns()),
            &[&invoice_id],
        )
        .await?;
    invoice_from_row(row)
}

async fn load_payout_account(
    transaction: &Transaction<'_>,
    payout_account_id: &str,
) -> Result<ProviderPayoutAccountRecord, SessionError> {
    let row = transaction
        .query_one(
            &format!(
                "{} WHERE payout_account_id = $1",
                payout_account_select_columns()
            ),
            &[&payout_account_id],
        )
        .await?;
    payout_account_from_row(row)
}

async fn load_active_payout_account(
    transaction: &Transaction<'_>,
    provider_id: &str,
    currency: &str,
) -> Result<ProviderPayoutAccountRecord, SessionError> {
    let row = transaction
        .query_opt(
            &format!(
                "{} WHERE provider_id = $1 AND currency = $2 AND status = 'active' ORDER BY updated_at DESC LIMIT 1 FOR UPDATE",
                payout_account_select_columns()
            ),
            &[&provider_id, &currency],
        )
        .await?
        .ok_or_else(|| SessionError::Conflict("provider has no active payout account".to_string()))?;
    payout_account_from_row(row)
}

async fn load_payout(
    transaction: &Transaction<'_>,
    payout_id: &str,
) -> Result<ProviderPayoutRecord, SessionError> {
    let row = transaction
        .query_one(
            &format!("{} WHERE payout_id = $1", payout_select_columns()),
            &[&payout_id],
        )
        .await?;
    payout_from_row(row)
}

async fn load_balances<C>(
    client: &C,
    owner_type: &str,
    owner_id: &str,
) -> Result<Vec<BillingBalance>, SessionError>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT account_type, currency, COALESCE(SUM(amount_micros), 0)::BIGINT AS balance_micros FROM financial_ledger_lines WHERE account_owner_type = $1 AND account_owner_id = $2 GROUP BY account_type, currency ORDER BY account_type, currency",
            &[&owner_type, &owner_id],
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| BillingBalance {
            account_type: row.get("account_type"),
            owner_type: owner_type.to_string(),
            owner_id: owner_id.to_string(),
            currency: row.get("currency"),
            balance_micros: row.get("balance_micros"),
        })
        .collect())
}

async fn account_balance<C>(
    client: &C,
    account_type: &str,
    owner_type: &str,
    owner_id: &str,
    currency: &str,
) -> Result<i64, SessionError>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "SELECT COALESCE(SUM(amount_micros), 0)::BIGINT AS balance_micros FROM financial_ledger_lines WHERE account_type = $1 AND account_owner_type = $2 AND account_owner_id = $3 AND currency = $4",
            &[&account_type, &owner_type, &owner_id, &currency],
        )
        .await?;
    Ok(row.get("balance_micros"))
}

async fn load_ledger_lines<C>(
    client: &C,
    owner_type: &str,
    owner_id: &str,
    limit: u32,
) -> Result<Vec<FinancialLedgerLineRecord>, SessionError>
where
    C: GenericClient + Sync,
{
    let limit = limit.clamp(1, MAX_FINANCIAL_LEDGER_LIMIT) as i64;
    let rows = client
        .query(
            &format!(
                "{} WHERE account_owner_type = $1 AND account_owner_id = $2 ORDER BY created_at DESC, line_number DESC LIMIT $3",
                ledger_select_columns()
            ),
            &[&owner_type, &owner_id, &limit],
        )
        .await?;
    rows.into_iter().map(ledger_line_from_row).collect()
}

fn price_from_row(row: Row) -> Result<MarketplacePriceRecord, SessionError> {
    Ok(MarketplacePriceRecord {
        price_id: row.get("price_id"),
        listing_id: row.get("listing_id"),
        schema_version: row.get("schema_version"),
        currency: row.get("currency"),
        price_per_hour_micros: from_i64_to_u64(row.get("price_per_hour_micros"))?,
        pricing_model: row.get("pricing_model"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn pix_from_row(row: Row) -> Result<PixPaymentIntentRecord, SessionError> {
    Ok(PixPaymentIntentRecord {
        payment_intent_id: row.get("payment_intent_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        schema_version: row.get("schema_version"),
        status: row.get("status"),
        amount_micros: from_i64_to_u64(row.get("amount_micros"))?,
        currency: row.get("currency"),
        provider: row.get("provider"),
        external_reference: row.get("external_reference"),
        adapter_status: row.get("adapter_status"),
        confirmed_at: row.get("confirmed_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn invoice_from_row(row: Row) -> Result<BillingInvoiceRecord, SessionError> {
    Ok(BillingInvoiceRecord {
        invoice_id: row.get("invoice_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        reservation_id: row.get("reservation_id"),
        usage_entry_id: row.get("usage_entry_id"),
        schema_version: row.get("schema_version"),
        status: row.get("status"),
        currency: row.get("currency"),
        subtotal_micros: from_i64_to_u64(row.get("subtotal_micros"))?,
        platform_fee_micros: from_i64_to_u64(row.get("platform_fee_micros"))?,
        provider_net_micros: from_i64_to_u64(row.get("provider_net_micros"))?,
        chargeback_reserve_micros: from_i64_to_u64(row.get("chargeback_reserve_micros"))?,
        total_micros: from_i64_to_u64(row.get("total_micros"))?,
        source_hash: row.get("source_hash"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn ledger_line_from_row(row: Row) -> Result<FinancialLedgerLineRecord, SessionError> {
    Ok(FinancialLedgerLineRecord {
        ledger_line_id: row.get("ledger_line_id"),
        transaction_id: row.get("transaction_id"),
        schema_version: row.get("schema_version"),
        line_number: from_i32_to_u32(row.get("line_number"))?,
        account_type: row.get("account_type"),
        account_owner_type: row.get("account_owner_type"),
        account_owner_id: row.get("account_owner_id"),
        currency: row.get("currency"),
        amount_micros: row.get("amount_micros"),
        source_type: row.get("source_type"),
        source_id: row.get("source_id"),
        description: row.get("description"),
        created_at: row.get("created_at"),
    })
}

fn payout_account_from_row(row: Row) -> Result<ProviderPayoutAccountRecord, SessionError> {
    Ok(ProviderPayoutAccountRecord {
        payout_account_id: row.get("payout_account_id"),
        provider_id: row.get("provider_id"),
        schema_version: row.get("schema_version"),
        payout_method: row.get("payout_method"),
        currency: row.get("currency"),
        pix_key_hash: row.get("pix_key_hash"),
        pix_key_last4: row.get("pix_key_last4"),
        kyc_status: row.get("kyc_status"),
        tax_status: row.get("tax_status"),
        minimum_payout_micros: from_i64_to_u64(row.get("minimum_payout_micros"))?,
        payout_hold_days: from_i32_to_u32(row.get("payout_hold_days"))?,
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn payout_from_row(row: Row) -> Result<ProviderPayoutRecord, SessionError> {
    Ok(ProviderPayoutRecord {
        payout_id: row.get("payout_id"),
        provider_id: row.get("provider_id"),
        payout_account_id: row.get("payout_account_id"),
        schema_version: row.get("schema_version"),
        status: row.get("status"),
        amount_micros: from_i64_to_u64(row.get("amount_micros"))?,
        currency: row.get("currency"),
        hold_until: row.get("hold_until"),
        external_reference: row.get("external_reference"),
        paid_at: row.get("paid_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn price_select_columns() -> &'static str {
    "SELECT price_id, listing_id, schema_version, currency, price_per_hour_micros, pricing_model, status, created_at, updated_at FROM marketplace_listing_prices"
}

fn pix_select_columns() -> &'static str {
    "SELECT payment_intent_id, organization_id, project_id, schema_version, status, amount_micros, currency, provider, external_reference, adapter_status, confirmed_at, created_at, updated_at FROM pix_payment_intents"
}

fn invoice_select_columns() -> &'static str {
    "SELECT invoice_id, organization_id, project_id, reservation_id, usage_entry_id, schema_version, status, currency, subtotal_micros, platform_fee_micros, provider_net_micros, chargeback_reserve_micros, total_micros, source_hash, created_at, updated_at FROM billing_invoices"
}

fn ledger_select_columns() -> &'static str {
    "SELECT ledger_line_id, transaction_id, schema_version, line_number, account_type, account_owner_type, account_owner_id, currency, amount_micros, source_type, source_id, description, created_at FROM financial_ledger_lines"
}

fn payout_account_select_columns() -> &'static str {
    "SELECT payout_account_id, provider_id, schema_version, payout_method, currency, pix_key_hash, pix_key_last4, kyc_status, tax_status, minimum_payout_micros, payout_hold_days, status, created_at, updated_at FROM provider_payout_accounts"
}

fn payout_select_columns() -> &'static str {
    "SELECT payout_id, provider_id, payout_account_id, schema_version, status, amount_micros, currency, hold_until, external_reference, paid_at, created_at, updated_at FROM provider_payouts"
}

fn idempotency_from_row(row: Row) -> IdempotencyRecord {
    IdempotencyRecord {
        request_hash: row.get("request_hash"),
        status_code: row.get::<_, i32>("status_code") as u16,
        response_json: row.get("response_json"),
    }
}

fn validate_price_request(request: &UpsertMarketplacePriceRequest) -> Result<(), SessionError> {
    validate_money(request.price_per_hour_micros, &request.currency)?;
    if let Some(model) = request.pricing_model.as_deref() {
        validate_id("pricing_model", model, 64)?;
        if model != "gpu_hour" {
            return Err(SessionError::Invalid(
                "only gpu_hour pricing is supported in BN-18".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_pix_intent_request(
    request: &CreatePixPaymentIntentRequest,
) -> Result<(), SessionError> {
    validate_money(request.amount_micros, &request.currency)?;
    if let Some(reference) = request.external_reference.as_deref() {
        validate_external_reference(reference)?;
    }
    Ok(())
}

fn validate_confirm_pix_request(
    request: &ConfirmPixPaymentIntentRequest,
) -> Result<(), SessionError> {
    validate_id("provider", &request.provider, 64)?;
    validate_external_reference(&request.external_reference)?;
    if let Some(paid_at) = request.paid_at.as_deref() {
        parse_timestamp("paid_at", paid_at)?;
    }
    Ok(())
}

fn validate_payout_account_request(
    request: &UpsertProviderPayoutAccountRequest,
) -> Result<(), SessionError> {
    if request.payout_method != "pix" {
        return Err(SessionError::Invalid(
            "only Pix payout accounts are supported in BN-18".to_string(),
        ));
    }
    validate_currency(&request.currency)?;
    if !is_bounded_ascii(&request.pix_key_hash, 160) || request.pix_key_hash.len() < 16 {
        return Err(SessionError::Invalid(
            "pix_key_hash must be a stored hash, not a raw Pix key".to_string(),
        ));
    }
    if !is_bounded_ascii(&request.pix_key_last4, 16) {
        return Err(SessionError::Invalid(
            "pix_key_last4 must be a short masked suffix".to_string(),
        ));
    }
    if !matches!(
        request.kyc_status.as_str(),
        "pending" | "verified" | "rejected"
    ) {
        return Err(SessionError::Invalid("unsupported KYC status".to_string()));
    }
    if !matches!(
        request.tax_status.as_str(),
        "pending" | "verified" | "blocked"
    ) {
        return Err(SessionError::Invalid("unsupported tax status".to_string()));
    }
    if request.minimum_payout_micros == Some(0) {
        return Err(SessionError::Invalid(
            "minimum payout must be greater than zero".to_string(),
        ));
    }
    if request.payout_hold_days.unwrap_or(DEFAULT_PAYOUT_HOLD_DAYS) > 90 {
        return Err(SessionError::Invalid(
            "payout hold cannot exceed 90 days".to_string(),
        ));
    }
    Ok(())
}

fn validate_money(amount_micros: u64, currency: &str) -> Result<(), SessionError> {
    if amount_micros == 0 {
        return Err(SessionError::Invalid(
            "amount_micros must be greater than zero".to_string(),
        ));
    }
    validate_currency(currency)
}

fn validate_currency(currency: &str) -> Result<(), SessionError> {
    if currency.len() == 3
        && currency
            .chars()
            .all(|character| character.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "currency must be a three-letter uppercase ISO code".to_string(),
        ))
    }
}

fn validate_bps(platform_fee_bps: u32, reserve_bps: u32) -> Result<(), SessionError> {
    if platform_fee_bps > 10_000 || reserve_bps > 10_000 {
        return Err(SessionError::Invalid(
            "basis points exceed 100%".to_string(),
        ));
    }
    if platform_fee_bps.saturating_add(reserve_bps) > 10_000 {
        return Err(SessionError::Invalid(
            "platform fee plus reserve cannot exceed 100%".to_string(),
        ));
    }
    Ok(())
}

fn billable_amount_micros(seconds: u64, price_per_hour_micros: u64) -> Result<u64, SessionError> {
    let raw = u128::from(seconds)
        .checked_mul(u128::from(price_per_hour_micros))
        .ok_or_else(|| SessionError::Invalid("billing amount overflow".to_string()))?;
    u64::try_from(raw.div_ceil(3600))
        .map_err(|_| SessionError::Invalid("billing amount overflow".to_string()))
}

fn settlement_amounts(
    subtotal_micros: u64,
    platform_fee_bps: u32,
    reserve_bps: u32,
) -> Result<SettlementAmounts, SessionError> {
    validate_bps(platform_fee_bps, reserve_bps)?;
    let platform_fee_micros = proportional_amount(subtotal_micros, platform_fee_bps)?;
    let chargeback_reserve_micros = proportional_amount(subtotal_micros, reserve_bps)?;
    let provider_net_micros = subtotal_micros
        .checked_sub(platform_fee_micros)
        .and_then(|remaining| remaining.checked_sub(chargeback_reserve_micros))
        .ok_or_else(|| SessionError::Invalid("settlement amount underflow".to_string()))?;
    Ok(SettlementAmounts {
        subtotal_micros,
        platform_fee_micros,
        provider_net_micros,
        chargeback_reserve_micros,
    })
}

fn proportional_amount(amount: u64, bps: u32) -> Result<u64, SessionError> {
    let value = u128::from(amount)
        .checked_mul(u128::from(bps))
        .ok_or_else(|| SessionError::Invalid("amount overflow".to_string()))?
        / 10_000;
    u64::try_from(value).map_err(|_| SessionError::Invalid("amount overflow".to_string()))
}

fn require_customer_scope(auth: &CustomerApiKeyAuth, scope: &str) -> Result<(), SessionError> {
    if auth.scopes.iter().any(|candidate| candidate == scope) {
        Ok(())
    } else {
        Err(SessionError::Unauthorized)
    }
}

fn validate_external_reference(reference: &str) -> Result<(), SessionError> {
    if is_bounded_ascii(reference, 160) {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "external reference must be printable ASCII".to_string(),
        ))
    }
}

fn validate_id(label: &str, value: &str, maximum_len: usize) -> Result<(), SessionError> {
    let valid = !value.trim().is_empty()
        && value.len() <= maximum_len
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
        });
    if valid {
        Ok(())
    } else {
        Err(SessionError::Invalid(format!(
            "{label} must be a short ASCII identifier"
        )))
    }
}

fn is_bounded_ascii(value: &str, maximum_len: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_len
        && value
            .chars()
            .all(|character| character.is_ascii() && !character.is_ascii_control())
}

fn parse_timestamp(label: &str, value: &str) -> Result<(), SessionError> {
    chrono::DateTime::parse_from_rfc3339(value).map_err(|error| {
        SessionError::Invalid(format!("{label} must be RFC3339 timestamp: {error}"))
    })?;
    Ok(())
}

fn to_i64(value: u64) -> Result<i64, SessionError> {
    i64::try_from(value).map_err(|_| SessionError::Invalid("amount overflow".to_string()))
}

fn from_i64_to_u64(value: i64) -> Result<u64, SessionError> {
    u64::try_from(value).map_err(|_| SessionError::Database(DbError::new("negative amount")))
}

fn from_i32_to_u32(value: i32) -> Result<u32, SessionError> {
    u32::try_from(value).map_err(|_| SessionError::Database(DbError::new("negative count")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_amount_rounds_up_to_micro_unit() {
        assert_eq!(
            billable_amount_micros(3600, 10_000_000).unwrap(),
            10_000_000
        );
        assert_eq!(billable_amount_micros(1, 3_600_000).unwrap(), 1_000);
    }

    #[test]
    fn settlement_splits_customer_total_into_provider_fee_and_reserve() {
        let amounts = settlement_amounts(10_000_000, 1500, 500).unwrap();
        assert_eq!(amounts.platform_fee_micros, 1_500_000);
        assert_eq!(amounts.chargeback_reserve_micros, 500_000);
        assert_eq!(amounts.provider_net_micros, 8_000_000);
    }

    #[test]
    fn financial_transaction_must_balance() {
        let balanced = vec![
            LedgerLine {
                account_type: "customer_balance",
                owner_type: "project",
                owner_id: Some("project_1".to_string()),
                amount_micros: -100,
                description: "charge",
            },
            LedgerLine {
                account_type: "provider_payable",
                owner_type: "provider",
                owner_id: Some("provider_1".to_string()),
                amount_micros: 100,
                description: "payable",
            },
        ];
        assert!(assert_balanced(&balanced).is_ok());
        let mut unbalanced = balanced;
        unbalanced[1].amount_micros = 99;
        assert!(assert_balanced(&unbalanced).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_billing_flow_settles_confirmed_balance_and_creates_payout() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_billing_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let client = db.connect().await.unwrap();
        client
            .batch_execute(
                r#"
                INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at)
                VALUES ('provider_billing', NULL, 'Billing Provider', 'available', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at)
                VALUES ('device_billing', 'provider_billing', 'machine_billing', 'active', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint)
                VALUES ('session_billing', 'provider_billing', 'device_billing', 'online', 0, '2026-07-13T00:00:00Z', '2026-07-13T01:00:00Z', 'fp_billing');
                INSERT INTO workload_policies (policy_id, policy_version, schema_version, workload_type, display_name, requirements_json, status, created_at, updated_at)
                VALUES ('policy_billing', '2026.07.0', 'burd-workload-policy-v2', 'llm_realtime_api', 'Billing policy', '{}', 'active', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO marketplace_listings (listing_id, provider_id, provider_display_name, device_id, session_id, schema_version, engine_version, status, current_status, workload_type, policy_id, policy_version, gpu_uuid, gpu_verified, gpu_verification_source, vram_total_mib, vram_verified, vram_verification_source, region, region_source, trust_score, risk_score, reliability_score, verification_status, proof_freshness_status, remote_network_score, effective_network_score, regional_reachability_json, benchmark_status, benchmark_metrics_json, price_source, availability_window_json, active_lease_count, reason_codes_json, source_hash, published_at, updated_at)
                VALUES ('listing_billing', 'provider_billing', 'Billing Provider', 'device_billing', 'session_billing', 'burd-marketplace-listing-v1', 'burd-marketplace-engine-v1', 'published', 'reserved', 'llm_realtime_api', 'policy_billing', '2026.07.0', 'GPU-billing', TRUE, 'proof', 24576, TRUE, 'benchmark', 'br-sao', 'probe', 90, 2, 99, 'verified', 'fresh', 90, 90, '[]', 'succeeded', '{}', 'not_configured_bn16', '{}', 0, '[]', 'listing_source_hash', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO organizations (organization_id, schema_version, display_name, status, created_at, updated_at)
                VALUES ('org_billing', 'burd-organization-v1', 'Billing Org', 'active', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO projects (project_id, organization_id, schema_version, display_name, status, created_at, updated_at)
                VALUES ('project_billing', 'org_billing', 'burd-project-v1', 'Billing Project', 'active', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO marketplace_reservations (reservation_id, organization_id, project_id, listing_id, provider_id, device_id, session_id, schema_version, workload_type, gpu_uuid, status, idempotency_key, request_hash, starts_at, expires_at, reserved_gpu_seconds, reason_codes_json, created_at, updated_at)
                VALUES ('reservation_billing', 'org_billing', 'project_billing', 'listing_billing', 'provider_billing', 'device_billing', 'session_billing', 'burd-marketplace-reservation-v1', 'llm_realtime_api', 'GPU-billing', 'reserved', 'reservation_key', 'reservation_hash', '2026-07-13T00:00:00Z', '2026-07-13T01:00:00Z', 3600, '[]', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO compute_jobs (job_id, provider_id, device_id, session_id, schema_version, workload_type, template_id, image_ref, gpu_uuid, backend, parameters_json, input_artifacts_json, expected_outputs_json, result_artifacts_json, result_metrics_json, status, timeout_seconds, created_at, assigned_at, accepted_at, started_at, completed_at, updated_at)
                VALUES ('job_billing', 'provider_billing', 'device_billing', 'session_billing', 'burd-job-v1', 'llm_realtime_api', 'llm_inference', 'ghcr.io/burd/runtime/llm@sha256:test', 'GPU-billing', 'cuda', '{}', '[]', '[]', '[]', '{}', 'succeeded', 900, '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z', '2026-07-13T00:10:00Z', '2026-07-13T00:10:00Z');
                INSERT INTO usage_ledger_entries (entry_id, schema_version, entry_type, job_id, provider_id, device_id, session_id, workload_type, gpu_uuid, job_status, job_started_at, job_completed_at, reserved_gpu_seconds, actual_gpu_seconds, billable_gpu_seconds, non_billable_gpu_seconds, idle_billable_gpu_seconds, idle_unbillable_gpu_seconds, input_bytes, output_bytes, network_transfer_bytes, storage_bytes, retry_count, provider_caused_failure, customer_caused_failure, challenge_non_billable_seconds, reason_codes_json, receipt_json, receipt_hash, receipt_signature_status, source_hash, created_at)
                VALUES ('usage_billing', 'burd-usage-ledger-v1', 'job_usage_finalized', 'job_billing', 'provider_billing', 'device_billing', 'session_billing', 'llm_realtime_api', 'GPU-billing', 'succeeded', '2026-07-13T00:00:00Z', '2026-07-13T00:10:00Z', 3600, 3600, 3600, 0, 0, 0, 0, 0, 0, 0, 0, FALSE, FALSE, 0, '[]', '{}', 'receipt_hash_billing', 'unsigned', 'usage_source_hash_billing', '2026-07-13T00:10:00Z');
                "#,
            )
            .await
            .unwrap();
        drop(client);

        db.upsert_marketplace_price(
            "req_price",
            "listing_billing",
            &UpsertMarketplacePriceRequest {
                currency: "BRL".to_string(),
                price_per_hour_micros: 10_000_000,
                pricing_model: None,
            },
        )
        .await
        .unwrap();

        let unfunded = db
            .settle_reservation_billing(
                "req_unfunded_settlement",
                "reservation_billing",
                &SettleReservationBillingRequest {
                    usage_entry_id: "usage_billing".to_string(),
                    platform_fee_bps: None,
                    chargeback_reserve_bps: None,
                },
            )
            .await;
        assert!(matches!(unfunded, Err(SessionError::Conflict(_))));

        let auth = CustomerApiKeyAuth {
            api_key_id: "api_key_billing".to_string(),
            organization_id: "org_billing".to_string(),
            project_id: Some("project_billing".to_string()),
            scopes: vec!["billing:read".to_string(), "billing:write".to_string()],
        };
        let created = db
            .create_pix_payment_intent_idempotently(CreatePixPaymentIntentCommand {
                request_id: "req_pix".to_string(),
                scope: "POST /v1/billing/projects/project_billing/pix/payment-intents".to_string(),
                idempotency_key: "pix_key_1".to_string(),
                request_hash: "pix_hash_1".to_string(),
                auth: auth.clone(),
                project_id: "project_billing".to_string(),
                request: CreatePixPaymentIntentRequest {
                    amount_micros: 10_000_000,
                    currency: "BRL".to_string(),
                    external_reference: None,
                },
            })
            .await
            .unwrap();
        let CreatePixPaymentIntentOutcome::Response(record) = created else {
            panic!("Pix intent should be idempotently stored");
        };
        let pix_response: PixPaymentIntentResponse =
            serde_json::from_str(&record.response_json).unwrap();
        let payment_intent_id = pix_response.payment_intent.payment_intent_id;
        db.confirm_pix_payment_intent(
            "req_confirm_pix",
            &payment_intent_id,
            &ConfirmPixPaymentIntentRequest {
                provider: "manual_pix".to_string(),
                external_reference: "pix_external_1".to_string(),
                paid_at: Some("2026-07-13T00:11:00Z".to_string()),
            },
        )
        .await
        .unwrap();

        let funded_balance = db
            .project_billing_balance("req_project_balance", &auth, "project_billing")
            .await
            .unwrap();
        assert_eq!(funded_balance.balances[0].balance_micros, 10_000_000);

        let invoice = db
            .settle_reservation_billing(
                "req_settlement",
                "reservation_billing",
                &SettleReservationBillingRequest {
                    usage_entry_id: "usage_billing".to_string(),
                    platform_fee_bps: None,
                    chargeback_reserve_bps: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(invoice.invoice.total_micros, 10_000_000);
        assert_eq!(invoice.invoice.provider_net_micros, 8_000_000);

        let project_balance = db
            .project_billing_balance("req_project_balance_after", &auth, "project_billing")
            .await
            .unwrap();
        assert_eq!(project_balance.balances[0].balance_micros, 0);
        let provider_balance = db
            .provider_billing_balance("req_provider_balance", "provider_billing")
            .await
            .unwrap();
        assert_eq!(provider_balance.balances[0].balance_micros, 8_000_000);

        db.upsert_provider_payout_account(
            "req_payout_account",
            "provider_billing",
            &UpsertProviderPayoutAccountRequest {
                payout_method: "pix".to_string(),
                currency: "BRL".to_string(),
                pix_key_hash: "hash_0123456789abcdef".to_string(),
                pix_key_last4: "1234".to_string(),
                kyc_status: "verified".to_string(),
                tax_status: "verified".to_string(),
                minimum_payout_micros: Some(1_000_000),
                payout_hold_days: Some(0),
            },
        )
        .await
        .unwrap();
        let payout = db
            .create_provider_payout(
                "req_payout",
                "provider_billing",
                &CreateProviderPayoutRequest {
                    amount_micros: 8_000_000,
                    currency: "BRL".to_string(),
                },
            )
            .await
            .unwrap();
        assert_eq!(payout.payout.status, "approved");

        let provider_after_payout = db
            .provider_billing_balance("req_provider_balance_after", "provider_billing")
            .await
            .unwrap();
        assert_eq!(provider_after_payout.balances[0].balance_micros, 0);
        db.drop_schema_for_test().await.unwrap();
    }
}
