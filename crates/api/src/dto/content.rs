use super::*;

/// A validated theme manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = ThemeManifest)]
pub struct ThemeManifestDto {
    /// Stable theme identifier.
    pub id: String,
    /// Human-readable theme name.
    pub name: String,
    /// Theme version.
    pub version: String,
    /// Platform and plugin capabilities the theme requires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    pub requires: Option<ThemeRequirements>,
}

impl From<ThemeManifest> for ThemeManifestDto {
    fn from(manifest: ThemeManifest) -> Self {
        Self {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            requires: manifest.requires,
        }
    }
}

impl From<ThemeManifestDto> for ThemeManifest {
    fn from(manifest: ThemeManifestDto) -> Self {
        Self {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            requires: manifest.requires,
        }
    }
}

/// A registered storefront theme as exposed to administrators.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Theme)]
pub struct ThemeDto {
    /// Theme identifier.
    pub id: String,
    /// Parsed theme manifest metadata.
    pub manifest: ThemeManifestDto,
    /// JSON Schema used to validate settings.
    pub settings_schema: Option<Value>,
    /// Current theme settings.
    pub settings: Value,
    /// Draft settings, when they differ from the live settings.
    pub draft_settings: Option<Value>,
    /// Monotonic settings revision.
    pub settings_revision: i64,
    /// Whether this is the active theme.
    pub is_active: bool,
    /// Creation timestamp.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp.
    #[schema(required = false)]
    pub updated_at: i64,
}

impl From<Theme> for ThemeDto {
    fn from(theme: Theme) -> Self {
        Self {
            id: theme.id.to_string(),
            manifest: theme.manifest.into(),
            settings_schema: theme.settings_schema,
            settings: theme.settings,
            draft_settings: theme.draft_settings,
            settings_revision: theme.settings_revision,
            is_active: theme.is_active,
            created_at: theme.created_at,
            updated_at: theme.updated_at,
        }
    }
}

/// The active storefront theme exposed to public storefronts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = StoreTheme)]
pub struct StoreThemeDto {
    /// Theme identifier.
    pub id: String,
    /// Public theme manifest metadata.
    pub manifest: ThemeManifestDto,
    /// Current theme settings.
    pub settings: Value,
}

impl From<Theme> for StoreThemeDto {
    fn from(theme: Theme) -> Self {
        Self {
            id: theme.id.to_string(),
            manifest: theme.manifest.into(),
            settings: theme.settings,
        }
    }
}

/// A published storefront content page.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct StorePageResponse {
    /// The requested page.
    pub page: Page,
}

/// A storefront navigation tree.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct NavigationResponse {
    /// The requested navigation tree.
    pub navigation: NavigationTree,
}

/// The active storefront theme.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct StoreThemeResponse {
    /// Active theme manifest and settings.
    pub theme: StoreThemeDto,
}
