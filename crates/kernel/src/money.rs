//! Monetary value types.
//!
//! Amounts are stored as integer minor units. Floating point is never used for
//! authoritative commercial values.

use serde::Deserialize;

use crate::error::KernelError;

/// A three-letter ISO-style currency code.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    /// Creates a validated uppercase currency code.
    pub fn new(code: impl Into<String>) -> Result<Self, KernelError> {
        let code = code.into().trim().to_ascii_uppercase();
        if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(KernelError::InvalidCurrency(code));
        }
        Ok(Self(code))
    }

    /// Returns the code as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CurrencyCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::str::FromStr for CurrencyCode {
    type Err = KernelError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for CurrencyCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// An amount in minor currency units with its currency.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Money {
    amount_minor: i64,
    currency: CurrencyCode,
}

impl Money {
    /// Creates a monetary value from integer minor units.
    #[must_use]
    pub fn new(amount_minor: i64, currency: CurrencyCode) -> Self {
        Self {
            amount_minor,
            currency,
        }
    }

    /// Returns the amount in minor units.
    #[must_use]
    pub fn amount_minor(&self) -> i64 {
        self.amount_minor
    }

    /// Returns the currency.
    #[must_use]
    pub fn currency(&self) -> &CurrencyCode {
        &self.currency
    }

    /// Adds two monetary values of the same currency.
    pub fn checked_add(&self, other: &Self) -> Result<Self, KernelError> {
        if self.currency != other.currency {
            return Err(KernelError::CurrencyMismatch {
                left: self.currency.to_string(),
                right: other.currency.to_string(),
            });
        }
        let amount_minor = self
            .amount_minor
            .checked_add(other.amount_minor)
            .ok_or_else(|| KernelError::Validation("money overflow".to_owned()))?;
        Ok(Self::new(amount_minor, self.currency.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::{CurrencyCode, Money};

    #[test]
    fn currency_codes_are_validated() {
        assert!(CurrencyCode::new("usd").is_ok());
        assert!(CurrencyCode::new("US").is_err());
        assert!(CurrencyCode::new("US1").is_err());
    }

    #[test]
    fn money_requires_matching_currencies() {
        let usd = CurrencyCode::new("USD").unwrap();
        let eur = CurrencyCode::new("EUR").unwrap();
        let left = Money::new(100, usd.clone());
        let right = Money::new(250, usd);

        assert_eq!(left.checked_add(&right).unwrap().amount_minor(), 350);
        assert!(left.checked_add(&Money::new(1, eur)).is_err());
    }
}
