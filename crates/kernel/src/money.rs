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

    /// Number of decimal digits used by the currency minor unit.
    ///
    /// Most currencies use two digits, but the platform must not assume that.
    #[must_use]
    pub fn minor_unit_digits(&self) -> u32 {
        match self.0.as_str() {
            "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF"
            | "UGX" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
            "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
            _ => 2,
        }
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

/// Rounding behaviour when applying a rate to a monetary amount.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rounding {
    /// Round half away from zero.
    #[default]
    HalfAwayFromZero,
    /// Round towards negative infinity.
    Floor,
    /// Round towards positive infinity.
    Ceil,
    /// Round towards zero.
    Truncate,
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

    /// Creates a zero amount in the given currency.
    #[must_use]
    pub fn zero(currency: CurrencyCode) -> Self {
        Self::new(0, currency)
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

    /// Returns true when the amount is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.amount_minor == 0
    }

    /// Returns true when the amount is negative.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.amount_minor < 0
    }

    /// Returns the amount with the opposite sign.
    #[must_use]
    pub fn negated(&self) -> Self {
        Self::new(-self.amount_minor, self.currency.clone())
    }

    /// Adds two monetary values of the same currency.
    pub fn checked_add(&self, other: &Self) -> Result<Self, KernelError> {
        self.ensure_same_currency(other)?;
        let amount_minor = self
            .amount_minor
            .checked_add(other.amount_minor)
            .ok_or_else(|| KernelError::Validation("money overflow".to_owned()))?;
        Ok(Self::new(amount_minor, self.currency.clone()))
    }

    /// Subtracts another monetary value of the same currency.
    pub fn checked_sub(&self, other: &Self) -> Result<Self, KernelError> {
        self.ensure_same_currency(other)?;
        let amount_minor = self
            .amount_minor
            .checked_sub(other.amount_minor)
            .ok_or_else(|| KernelError::Validation("money overflow".to_owned()))?;
        Ok(Self::new(amount_minor, self.currency.clone()))
    }

    /// Multiplies the amount by an integer factor.
    pub fn checked_mul(&self, factor: i64) -> Result<Self, KernelError> {
        let amount_minor = self
            .amount_minor
            .checked_mul(factor)
            .ok_or_else(|| KernelError::Validation("money overflow".to_owned()))?;
        Ok(Self::new(amount_minor, self.currency.clone()))
    }

    /// Applies a rate expressed in basis points (1/100 of a percent).
    ///
    /// For example, `1250` basis points is `12.5%`.
    pub fn apply_basis_points(
        &self,
        basis_points: i64,
        rounding: Rounding,
    ) -> Result<Self, KernelError> {
        let product = i128::from(self.amount_minor) * i128::from(basis_points);
        let divisor = 10_000_i128;
        let quotient = product / divisor;
        let remainder = product % divisor;
        let amount = if remainder == 0 {
            quotient
        } else {
            match rounding {
                Rounding::Truncate => quotient,
                Rounding::Floor => {
                    if product < 0 && remainder != 0 {
                        quotient - 1
                    } else {
                        quotient
                    }
                }
                Rounding::Ceil => {
                    if product > 0 && remainder != 0 {
                        quotient + 1
                    } else {
                        quotient
                    }
                }
                Rounding::HalfAwayFromZero => {
                    if remainder.abs() * 2 >= divisor {
                        if product < 0 {
                            quotient - 1
                        } else {
                            quotient + 1
                        }
                    } else {
                        quotient
                    }
                }
            }
        };
        let amount_minor = i64::try_from(amount)
            .map_err(|_| KernelError::Validation("money overflow".to_owned()))?;
        Ok(Self::new(amount_minor, self.currency.clone()))
    }

    /// Clamps the amount into the given range.
    #[must_use]
    pub fn clamp(&self, minimum: i64, maximum: i64) -> Self {
        Self::new(
            self.amount_minor.clamp(minimum, maximum),
            self.currency.clone(),
        )
    }

    fn ensure_same_currency(&self, other: &Self) -> Result<(), KernelError> {
        if self.currency != other.currency {
            return Err(KernelError::CurrencyMismatch {
                left: self.currency.to_string(),
                right: other.currency.to_string(),
            });
        }
        Ok(())
    }
}

impl std::fmt::Display for Money {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digits = self.currency.minor_unit_digits();
        if digits == 0 {
            return write!(formatter, "{} {}", self.amount_minor, self.currency);
        }
        let divisor = 10_i64.pow(digits);
        let major = self.amount_minor / divisor;
        let minor = (self.amount_minor % divisor).abs();
        write!(
            formatter,
            "{}.{:0width$} {}",
            major,
            minor,
            self.currency,
            width = digits as usize
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{CurrencyCode, Money, Rounding};

    fn usd() -> CurrencyCode {
        CurrencyCode::new("usd").unwrap()
    }

    #[test]
    fn currency_codes_are_validated() {
        assert!(CurrencyCode::new("usd").is_ok());
        assert!(CurrencyCode::new("US").is_err());
        assert!(CurrencyCode::new("US1").is_err());
    }

    #[test]
    fn currency_minor_units_are_not_assumed_to_be_two() {
        assert_eq!(CurrencyCode::new("JPY").unwrap().minor_unit_digits(), 0);
        assert_eq!(CurrencyCode::new("KWD").unwrap().minor_unit_digits(), 3);
        assert_eq!(usd().minor_unit_digits(), 2);
    }

    #[test]
    fn money_requires_matching_currencies() {
        let eur = CurrencyCode::new("EUR").unwrap();
        let left = Money::new(100, usd());
        let right = Money::new(250, usd());

        assert_eq!(left.checked_add(&right).unwrap().amount_minor(), 350);
        assert!(left.checked_add(&Money::new(1, eur)).is_err());
    }

    #[test]
    fn basis_points_round_half_away_from_zero() {
        let amount = Money::new(1005, usd());
        assert_eq!(
            amount
                .apply_basis_points(500, Rounding::HalfAwayFromZero)
                .unwrap()
                .amount_minor(),
            50
        );
        assert_eq!(
            Money::new(1015, usd())
                .apply_basis_points(500, Rounding::HalfAwayFromZero)
                .unwrap()
                .amount_minor(),
            51
        );
        assert_eq!(
            Money::new(1015, usd())
                .apply_basis_points(500, Rounding::Floor)
                .unwrap()
                .amount_minor(),
            50
        );
    }

    #[test]
    fn money_formats_with_currency_minor_units() {
        assert_eq!(Money::new(1234, usd()).to_string(), "12.34 USD");
        assert_eq!(
            Money::new(1234, CurrencyCode::new("JPY").unwrap()).to_string(),
            "1234 JPY"
        );
    }
}
