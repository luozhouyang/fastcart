//! Campaign budgets and authoring inputs.

use super::*;

/// How a campaign budget is consumed.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignBudgetType {
    /// Each promotion usage consumes one unit.
    #[default]
    Usage,
    /// Each usage consumes the discounted amount in the campaign currency.
    Spend,
}

impl CampaignBudgetType {
    /// Database representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Spend => "spend",
        }
    }
}

impl std::str::FromStr for CampaignBudgetType {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "usage" => Ok(Self::Usage),
            "spend" => Ok(Self::Spend),
            other => Err(AppError::validation(format!(
                "invalid campaign budget type: {other}"
            ))),
        }
    }
}

/// A campaign grouping promotions under a shared budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Campaign {
    /// Campaign identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the budget counts usages or monetary spend.
    pub budget_type: CampaignBudgetType,
    /// Currency of a monetary budget; absent for usage budgets.
    pub currency_code: Option<CurrencyCode>,
    /// Maximum usages or spend in minor units, or unlimited.
    pub budget_limit: Option<i64>,
    /// Usages or spend already consumed.
    pub budget_used: i64,
    /// Start of the validity window.
    pub starts_at: Option<i64>,
    /// End of the validity window.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl Campaign {
    /// Consumed budget expressed as a monetary amount, when this is a spend
    /// campaign.
    #[must_use]
    pub fn budget_spent(&self) -> Option<Money> {
        match (self.budget_type, self.currency_code.clone()) {
            (CampaignBudgetType::Spend, Some(currency)) => {
                Some(Money::new(self.budget_used, currency))
            }
            _ => None,
        }
    }
}

/// Input for creating a campaign.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateCampaignInput {
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Budget accounting mode; defaults to usage.
    #[serde(default)]
    pub budget_type: CampaignBudgetType,
    /// Currency required for spend budgets.
    #[serde(default)]
    pub currency_code: Option<CurrencyCode>,
    /// Maximum usages or spend in minor units.
    pub budget_limit: Option<i64>,
    /// Start timestamp.
    pub starts_at: Option<i64>,
    /// End timestamp.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
}

/// Input for partially updating a campaign.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateCampaignInput {
    /// New name.
    pub name: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New budget accounting mode.
    pub budget_type: Option<CampaignBudgetType>,
    /// New budget currency; `Some(None)` clears it.
    pub currency_code: Option<Option<CurrencyCode>>,
    /// New budget limit; `Some(None)` makes it unlimited.
    pub budget_limit: Option<Option<i64>>,
    /// New start.
    pub starts_at: Option<Option<i64>>,
    /// New end.
    pub ends_at: Option<Option<i64>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Validates the campaign budget fields.
pub(crate) fn validate_campaign_budget(
    budget_type: CampaignBudgetType,
    currency_code: Option<&CurrencyCode>,
    budget_limit: Option<i64>,
) -> Result<(), AppError> {
    if budget_limit.is_some_and(|limit| limit <= 0) {
        return Err(AppError::validation("campaign budget must be positive"));
    }
    match budget_type {
        CampaignBudgetType::Spend => {
            if currency_code.is_none() {
                return Err(AppError::validation(
                    "spend campaign budgets require a currency_code",
                ));
            }
            if budget_limit.is_none() {
                return Err(AppError::validation(
                    "spend campaign budgets require a budget_limit",
                ));
            }
        }
        CampaignBudgetType::Usage => {
            if currency_code.is_some() {
                return Err(AppError::validation(
                    "usage campaign budgets must not set currency_code",
                ));
            }
        }
    }
    Ok(())
}
