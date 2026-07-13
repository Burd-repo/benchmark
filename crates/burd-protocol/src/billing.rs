use serde::{Deserialize, Serialize};

pub const MARKETPLACE_PRICE_SCHEMA_VERSION: &str = "burd-marketplace-price-v1";
pub const FINANCIAL_LEDGER_SCHEMA_VERSION: &str = "burd-financial-ledger-v1";
pub const BILLING_INVOICE_SCHEMA_VERSION: &str = "burd-billing-invoice-v1";
pub const PIX_PAYMENT_INTENT_SCHEMA_VERSION: &str = "burd-pix-payment-intent-v1";
pub const PROVIDER_PAYOUT_ACCOUNT_SCHEMA_VERSION: &str = "burd-provider-payout-account-v1";
pub const PROVIDER_PAYOUT_SCHEMA_VERSION: &str = "burd-provider-payout-v1";
pub const BILLING_REFUND_SCHEMA_VERSION: &str = "burd-billing-refund-v1";
pub const BILLING_DISPUTE_SCHEMA_VERSION: &str = "burd-billing-dispute-v1";
pub const BILLING_RECONCILIATION_SCHEMA_VERSION: &str = "burd-billing-reconciliation-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertMarketplacePriceRequest {
    pub currency: String,
    pub price_per_hour_micros: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketplacePriceRecord {
    pub price_id: String,
    pub listing_id: String,
    pub schema_version: String,
    pub currency: String,
    pub price_per_hour_micros: u64,
    pub pricing_model: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketplacePriceResponse {
    pub request_id: String,
    pub price: MarketplacePriceRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatePixPaymentIntentRequest {
    pub amount_micros: u64,
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmPixPaymentIntentRequest {
    pub provider: String,
    pub external_reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PixPaymentIntentRecord {
    pub payment_intent_id: String,
    pub organization_id: String,
    pub project_id: String,
    pub schema_version: String,
    pub status: String,
    pub amount_micros: u64,
    pub currency: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_reference: Option<String>,
    pub adapter_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PixPaymentIntentResponse {
    pub request_id: String,
    pub payment_intent: PixPaymentIntentRecord,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettleReservationBillingRequest {
    pub usage_entry_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_fee_bps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chargeback_reserve_bps: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingInvoiceRecord {
    pub invoice_id: String,
    pub organization_id: String,
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_entry_id: Option<String>,
    pub schema_version: String,
    pub status: String,
    pub currency: String,
    pub subtotal_micros: u64,
    pub platform_fee_micros: u64,
    pub provider_net_micros: u64,
    pub chargeback_reserve_micros: u64,
    pub total_micros: u64,
    pub source_hash: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingInvoiceResponse {
    pub request_id: String,
    pub invoice: BillingInvoiceRecord,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinancialLedgerLineRecord {
    pub ledger_line_id: String,
    pub transaction_id: String,
    pub schema_version: String,
    pub line_number: u32,
    pub account_type: String,
    pub account_owner_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_owner_id: Option<String>,
    pub currency: String,
    pub amount_micros: i64,
    pub source_type: String,
    pub source_id: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinancialLedgerResponse {
    pub request_id: String,
    pub lines: Vec<FinancialLedgerLineRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingBalance {
    pub account_type: String,
    pub owner_type: String,
    pub owner_id: String,
    pub currency: String,
    pub balance_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingBalanceResponse {
    pub request_id: String,
    pub balances: Vec<BillingBalance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertProviderPayoutAccountRequest {
    pub payout_method: String,
    pub currency: String,
    pub pix_key_hash: String,
    pub pix_key_last4: String,
    pub kyc_status: String,
    pub tax_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_payout_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payout_hold_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderPayoutAccountRecord {
    pub payout_account_id: String,
    pub provider_id: String,
    pub schema_version: String,
    pub payout_method: String,
    pub currency: String,
    pub pix_key_hash: String,
    pub pix_key_last4: String,
    pub kyc_status: String,
    pub tax_status: String,
    pub minimum_payout_micros: u64,
    pub payout_hold_days: u32,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderPayoutAccountResponse {
    pub request_id: String,
    pub payout_account: ProviderPayoutAccountRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateProviderPayoutRequest {
    pub amount_micros: u64,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkProviderPayoutPaidRequest {
    pub external_reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderPayoutRecord {
    pub payout_id: String,
    pub provider_id: String,
    pub payout_account_id: String,
    pub schema_version: String,
    pub status: String,
    pub amount_micros: u64,
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderPayoutResponse {
    pub request_id: String,
    pub payout: ProviderPayoutRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateBillingRefundRequest {
    pub amount_micros: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingRefundRecord {
    pub refund_id: String,
    pub invoice_id: String,
    pub organization_id: String,
    pub project_id: String,
    pub schema_version: String,
    pub status: String,
    pub amount_micros: u64,
    pub currency: String,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingRefundResponse {
    pub request_id: String,
    pub refund: BillingRefundRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateBillingDisputeRequest {
    pub reason: String,
    pub hold_amount_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingDisputeRecord {
    pub dispute_id: String,
    pub invoice_id: String,
    pub organization_id: String,
    pub project_id: String,
    pub schema_version: String,
    pub status: String,
    pub reason: String,
    pub hold_amount_micros: u64,
    pub currency: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillingDisputeResponse {
    pub request_id: String,
    pub dispute: BillingDisputeRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateReconciliationEventRequest {
    pub provider: String,
    pub external_reference: String,
    pub amount_micros: u64,
    pub currency: String,
    pub event_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationEventRecord {
    pub reconciliation_event_id: String,
    pub schema_version: String,
    pub provider: String,
    pub external_reference: String,
    pub amount_micros: u64,
    pub currency: String,
    pub event_type: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationEventResponse {
    pub request_id: String,
    pub event: ReconciliationEventRecord,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pix_intent_omits_external_reference_until_adapter_sets_one() {
        let request = CreatePixPaymentIntentRequest {
            amount_micros: 10_000,
            currency: "BRL".to_string(),
            external_reference: None,
        };
        let serialized = serde_json::to_value(request).unwrap();
        assert_eq!(serialized["currency"], "BRL");
        assert!(serialized.get("external_reference").is_none());
    }

    #[test]
    fn ledger_line_amounts_are_signed() {
        let line = FinancialLedgerLineRecord {
            ledger_line_id: "line_1".to_string(),
            transaction_id: "txn_1".to_string(),
            schema_version: FINANCIAL_LEDGER_SCHEMA_VERSION.to_string(),
            line_number: 1,
            account_type: "customer_balance".to_string(),
            account_owner_type: "project".to_string(),
            account_owner_id: Some("project_1".to_string()),
            currency: "BRL".to_string(),
            amount_micros: -100,
            source_type: "invoice".to_string(),
            source_id: "invoice_1".to_string(),
            description: "customer charge".to_string(),
            created_at: "2026-07-13T00:00:00Z".to_string(),
        };
        assert_eq!(line.amount_micros, -100);
    }
}
