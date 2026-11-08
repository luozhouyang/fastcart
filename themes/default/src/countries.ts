/** A country selectable as a shipping destination. */
export interface CountryOption {
  code: string;
  label: string;
}

/**
 * The destination used before the customer supplies a shipping address.
 *
 * Storefront cart mutations require a destination country, so this single
 * constant is the fallback for the first add and for every mutation until the
 * details form records a different country.
 */
export const DEFAULT_COUNTRY_CODE = "US";

/** Small, dependency-free destination list for the checkout form. */
export const COUNTRIES: readonly CountryOption[] = [
  { code: "US", label: "United States" },
  { code: "CA", label: "Canada" },
  { code: "MX", label: "Mexico" },
  { code: "GB", label: "United Kingdom" },
  { code: "IE", label: "Ireland" },
  { code: "DE", label: "Germany" },
  { code: "FR", label: "France" },
  { code: "NL", label: "Netherlands" },
  { code: "BE", label: "Belgium" },
  { code: "AT", label: "Austria" },
  { code: "CH", label: "Switzerland" },
  { code: "ES", label: "Spain" },
  { code: "PT", label: "Portugal" },
  { code: "IT", label: "Italy" },
  { code: "SE", label: "Sweden" },
  { code: "NO", label: "Norway" },
  { code: "DK", label: "Denmark" },
  { code: "FI", label: "Finland" },
  { code: "PL", label: "Poland" },
  { code: "CZ", label: "Czechia" },
  { code: "AU", label: "Australia" },
  { code: "NZ", label: "New Zealand" },
  { code: "JP", label: "Japan" },
  { code: "KR", label: "South Korea" },
  { code: "SG", label: "Singapore" },
  { code: "HK", label: "Hong Kong" },
  { code: "TW", label: "Taiwan" },
  { code: "IN", label: "India" },
  { code: "AE", label: "United Arab Emirates" },
  { code: "ZA", label: "South Africa" },
  { code: "BR", label: "Brazil" },
];

/** Normalizes a destination code and falls back to the default country. */
export function normalizeCountryCode(value: string | null | undefined): string {
  const code = value?.trim().toUpperCase() ?? "";
  return /^[A-Z]{2}$/.test(code) ? code : DEFAULT_COUNTRY_CODE;
}

/** Returns the display label for a country code, or the code itself. */
export function countryLabel(code: string | null | undefined): string {
  if (!code) {
    return "";
  }
  const normalized = code.trim().toUpperCase();
  return COUNTRIES.find((country) => country.code === normalized)?.label ?? normalized;
}
