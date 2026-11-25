//! Manifest validation.

use super::*;

impl PluginManifest {
    /// Validates structural rules that do not depend on the archive.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(PluginError::InvalidManifest(format!(
                "unsupported schema_version: {}",
                self.schema_version
            )));
        }
        if self.api_version != HOST_API_VERSION {
            return Err(PluginError::UnsupportedApi(self.api_version));
        }
        if !valid_identifier(&self.id) {
            return Err(PluginError::InvalidManifest(format!(
                "invalid plugin id: {}",
                self.id
            )));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            return Err(PluginError::InvalidManifest(
                "plugin name must be between 1 and 120 characters".to_owned(),
            ));
        }
        if Version::parse(&self.version).is_err() {
            return Err(PluginError::InvalidManifest(format!(
                "invalid semantic version: {}",
                self.version
            )));
        }
        if let Some(description) = &self.description
            && description.chars().count() > 500
        {
            return Err(PluginError::InvalidManifest(
                "description exceeds 500 characters".to_owned(),
            ));
        }
        self.validate_runtime()?;
        self.validate_entry()?;
        self.validate_capabilities()?;
        self.validate_extensions()?;
        self.validate_contributions()?;
        self.validate_settings()?;
        self.validate_jobs()?;
        Ok(())
    }
}

impl PluginManifest {
    fn validate_runtime(&self) -> Result<(), PluginError> {
        if self.runtime.engine != "workerd" {
            return Err(PluginError::InvalidManifest(format!(
                "unsupported runtime engine: {}",
                self.runtime.engine
            )));
        }
        if !valid_date(&self.runtime.compatibility_date) {
            return Err(PluginError::InvalidManifest(
                "compatibility_date must be YYYY-MM-DD".to_owned(),
            ));
        }
        if self.runtime.compatibility_date.as_str() < MIN_COMPATIBILITY_DATE
            || self.runtime.compatibility_date.as_str() > MAX_COMPATIBILITY_DATE
        {
            return Err(PluginError::InvalidManifest(format!(
                "compatibility_date must be between {MIN_COMPATIBILITY_DATE} and {MAX_COMPATIBILITY_DATE}"
            )));
        }
        let mut seen = BTreeSet::new();
        for flag in &self.runtime.compatibility_flags {
            if !ALLOWED_COMPATIBILITY_FLAGS.contains(&flag.as_str()) {
                return Err(PluginError::InvalidManifest(format!(
                    "unsupported compatibility flag: {flag}"
                )));
            }
            if !seen.insert(flag) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate compatibility flag: {flag}"
                )));
            }
        }
        Ok(())
    }
}

impl PluginManifest {
    fn validate_entry(&self) -> Result<(), PluginError> {
        if self.entry.kind != "worker" {
            return Err(PluginError::InvalidManifest(format!(
                "unsupported entry type: {}",
                self.entry.kind
            )));
        }
        validate_relative_path(&self.entry.path, &["js", "mjs"])?;
        if let Some(export) = &self.entry.export
            && !valid_js_identifier(export)
        {
            return Err(PluginError::InvalidManifest(format!(
                "invalid entry export: {export}"
            )));
        }
        Ok(())
    }
}

impl PluginManifest {
    fn validate_capabilities(&self) -> Result<(), PluginError> {
        for value in &self.capabilities.data {
            parse_scope("data", value)?;
        }
        for value in &self.capabilities.storage {
            parse_scope("storage", value)?;
        }
        for value in &self.capabilities.secrets {
            parse_scope("secrets", value)?;
        }
        for value in &self.capabilities.storefront {
            parse_scope("storefront", value)?;
        }
        if self.capabilities.events.len() > MAX_EVENTS {
            return Err(PluginError::InvalidManifest(format!(
                "at most {MAX_EVENTS} event subscriptions are allowed"
            )));
        }
        let mut events = BTreeSet::new();
        for event in &self.capabilities.events {
            if !valid_event_name(event) {
                return Err(PluginError::InvalidManifest(format!(
                    "invalid event name: {event}"
                )));
            }
            if !events.insert(event) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate event subscription: {event}"
                )));
            }
        }
        if self.capabilities.network.len() > MAX_EGRESS_HOSTS {
            return Err(PluginError::InvalidManifest(format!(
                "at most {MAX_EGRESS_HOSTS} egress hosts are allowed"
            )));
        }
        let mut hosts = BTreeSet::new();
        for host in &self.capabilities.network {
            if !valid_hostname(host) {
                return Err(PluginError::InvalidManifest(format!(
                    "invalid egress host: {host}"
                )));
            }
            if !hosts.insert(host) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate egress host: {host}"
                )));
            }
        }
        for host in &self.capabilities.network_auth {
            if !valid_hostname(host) || !self.capabilities.network.contains(host) {
                return Err(PluginError::InvalidManifest(format!(
                    "network_auth host must also be allowlisted: {host}"
                )));
            }
        }
        for capability in &self.capabilities.admin {
            if !matches!(capability.as_str(), "admin.ui" | "plugins.settings") {
                return Err(PluginError::UnsupportedScope(capability.clone()));
            }
        }
        Ok(())
    }
}

impl PluginManifest {
    fn validate_extensions(&self) -> Result<(), PluginError> {
        if self.extensions.admin.len() > MAX_ADMIN_EXTENSIONS {
            return Err(PluginError::InvalidManifest(format!(
                "at most {MAX_ADMIN_EXTENSIONS} admin extensions are allowed"
            )));
        }
        let mut targets = BTreeSet::new();
        for extension in &self.extensions.admin {
            validate_target(&extension.target)?;
            if !targets.insert(extension.target.as_str()) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate admin target: {}",
                    extension.target
                )));
            }
            if let Some(height) = extension.height
                && !(80..=1200).contains(&height)
            {
                return Err(PluginError::InvalidManifest(
                    "admin extension height must be between 80 and 1200".to_owned(),
                ));
            }
            if let Some(priority) = extension.priority
                && !(-100..=100).contains(&priority)
            {
                return Err(PluginError::InvalidManifest(
                    "admin extension priority must be between -100 and 100".to_owned(),
                ));
            }
            if extension.target == "admin.nav.section" {
                let label = extension.label.as_deref().unwrap_or_default();
                if label.trim().is_empty() || label.chars().count() > 40 {
                    return Err(PluginError::InvalidManifest(
                        "nav extensions require a label of at most 40 characters".to_owned(),
                    ));
                }
                let route = extension.route.as_deref().unwrap_or_default();
                if !route.starts_with('/') || route.contains("..") {
                    return Err(PluginError::InvalidManifest(
                        "nav extensions require a site-relative route".to_owned(),
                    ));
                }
            } else if is_page_target(&extension.target) {
                let entry = extension.entry.as_deref().ok_or_else(|| {
                    PluginError::InvalidManifest(format!(
                        "target {} requires an entry",
                        extension.target
                    ))
                })?;
                validate_relative_path(entry, &["html", "htm"])?;
            }
        }

        if self.extensions.storefront.len() > MAX_STOREFRONT_EXTENSIONS {
            return Err(PluginError::InvalidManifest(format!(
                "at most {MAX_STOREFRONT_EXTENSIONS} storefront extensions are allowed"
            )));
        }
        for extension in &self.extensions.storefront {
            if extension.kind != "embed" {
                return Err(PluginError::InvalidManifest(format!(
                    "unsupported storefront extension kind: {}",
                    extension.kind
                )));
            }
            if !matches!(
                extension.placement.as_str(),
                "head:start" | "head:end" | "body:start" | "body:end"
            ) {
                return Err(PluginError::InvalidManifest(format!(
                    "unsupported storefront placement: {}",
                    extension.placement
                )));
            }
            validate_relative_path(&extension.entry, &["js", "mjs"])?;
            if let Some(css) = &extension.css {
                validate_relative_path(css, &["css"])?;
            }
            if extension.pages.is_empty() {
                return Err(PluginError::InvalidManifest(
                    "storefront pages must not be empty".to_owned(),
                ));
            }
            for page in &extension.pages {
                if page != "*" && !page.starts_with('/') {
                    return Err(PluginError::InvalidManifest(format!(
                        "invalid page matcher: {page}"
                    )));
                }
            }
            if let Some(height) = extension.height_hint
                && height > 2000
            {
                return Err(PluginError::InvalidManifest(
                    "height_hint must be at most 2000".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

impl PluginManifest {
    fn validate_contributions(&self) -> Result<(), PluginError> {
        if self.provides.len() > MAX_CONTRIBUTIONS {
            return Err(PluginError::InvalidManifest(format!(
                "at most {MAX_CONTRIBUTIONS} contributions are allowed"
            )));
        }
        let component_scope = self
            .capabilities
            .storefront
            .iter()
            .any(|scope| scope == "storefront.component");
        let mut seen = BTreeSet::new();
        let mut requires_component_scope = false;
        for contribution in &self.provides {
            let (kind, id, version) = match contribution {
                Contribution::StorefrontCapability(capability) => {
                    if !valid_identifier(&capability.id) {
                        return Err(PluginError::InvalidManifest(format!(
                            "invalid capability contribution id: {}",
                            capability.id
                        )));
                    }
                    if capability.version == 0 {
                        return Err(PluginError::InvalidManifest(format!(
                            "capability contribution {} version must be at least 1",
                            capability.id
                        )));
                    }
                    validate_contribution_schema(
                        &capability.config_schema,
                        "capability config_schema",
                    )?;
                    (
                        contribution.kind(),
                        capability.id.as_str(),
                        Some(capability.version),
                    )
                }
                Contribution::StorefrontComponent(component) => {
                    if !valid_identifier(&component.id) {
                        return Err(PluginError::InvalidManifest(format!(
                            "invalid component contribution id: {}",
                            component.id
                        )));
                    }
                    validate_contribution_object(&component.editor_manifest, "editor_manifest")?;
                    if let Some(entry) = &component.entry {
                        validate_relative_path(entry, &["js", "mjs"])?;
                    }
                    let mut requirement_ids = BTreeSet::new();
                    for requirement in &component.requires {
                        if !valid_identifier(&requirement.id) {
                            return Err(PluginError::InvalidManifest(format!(
                                "invalid capability requirement id: {}",
                                requirement.id
                            )));
                        }
                        if requirement.version == Some(0) {
                            return Err(PluginError::InvalidManifest(format!(
                                "capability requirement {} version must be at least 1",
                                requirement.id
                            )));
                        }
                        if !requirement_ids.insert(requirement.id.as_str()) {
                            return Err(PluginError::InvalidManifest(format!(
                                "duplicate capability requirement: {}",
                                requirement.id
                            )));
                        }
                    }
                    requires_component_scope = true;
                    (contribution.kind(), component.id.as_str(), None)
                }
            };
            if !seen.insert((kind, id, version)) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate {kind} contribution: {id}"
                )));
            }
        }
        if requires_component_scope && !component_scope {
            return Err(PluginError::InvalidManifest(
                "component contributions require the storefront.component scope".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_contribution_schema(schema: &Option<Value>, field: &str) -> Result<(), PluginError> {
    let Some(schema) = schema else {
        return Ok(());
    };
    if !schema.is_object() {
        return Err(PluginError::InvalidManifest(format!(
            "{field} must be a JSON object"
        )));
    }
    jsonschema::validator_for(schema)
        .map_err(|error| PluginError::InvalidManifest(format!("invalid {field}: {error}")))?;
    Ok(())
}

fn validate_contribution_object(value: &Option<Value>, field: &str) -> Result<(), PluginError> {
    if let Some(value) = value
        && !value.is_object()
    {
        return Err(PluginError::InvalidManifest(format!(
            "{field} must be a JSON object"
        )));
    }
    Ok(())
}

impl PluginManifest {
    fn validate_settings(&self) -> Result<(), PluginError> {
        let Some(settings) = &self.settings else {
            return Ok(());
        };
        let object = settings.schema.as_object().ok_or_else(|| {
            PluginError::InvalidManifest("settings.schema must be a JSON object".to_owned())
        })?;
        if object.get("type").and_then(Value::as_str) != Some("object") {
            return Err(PluginError::InvalidManifest(
                "settings.schema must declare type: object".to_owned(),
            ));
        }
        jsonschema::validator_for(&settings.schema).map_err(|error| {
            PluginError::InvalidManifest(format!("invalid settings schema: {error}"))
        })?;
        if let Some(ui) = &settings.ui {
            validate_relative_path(ui, &["html", "htm"])?;
        }
        Ok(())
    }
}

impl PluginManifest {
    fn validate_jobs(&self) -> Result<(), PluginError> {
        if self.jobs.len() > MAX_JOBS {
            return Err(PluginError::InvalidManifest(format!(
                "at most {MAX_JOBS} jobs are allowed"
            )));
        }
        let mut names = BTreeSet::new();
        if self.rpc.len() > 50 {
            return Err(PluginError::InvalidManifest(
                "at most 50 RPC methods may be declared".to_owned(),
            ));
        }
        let mut seen_rpc = std::collections::HashSet::new();
        for method in &self.rpc {
            if !valid_rpc_method(method) {
                return Err(PluginError::InvalidManifest(format!(
                    "invalid RPC method: {method}"
                )));
            }
            if !seen_rpc.insert(method) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate RPC method: {method}"
                )));
            }
        }
        for (method, permission) in &self.rpc_permissions {
            if !self.rpc.iter().any(|declared| declared == method) {
                return Err(PluginError::InvalidManifest(format!(
                    "RPC permission names an undeclared method: {method}"
                )));
            }
            if !valid_permission(permission) {
                return Err(PluginError::InvalidManifest(format!(
                    "invalid permission for RPC method {method}: {permission}"
                )));
            }
        }
        for job in &self.jobs {
            if !valid_identifier(&job.name) {
                return Err(PluginError::InvalidManifest(format!(
                    "invalid job name: {}",
                    job.name
                )));
            }
            if !names.insert(job.name.as_str()) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate job name: {}",
                    job.name
                )));
            }
            if let Some(schedule) = &job.schedule
                && !valid_cron(schedule)
            {
                return Err(PluginError::InvalidManifest(format!(
                    "invalid cron schedule for job {}: {schedule}",
                    job.name
                )));
            }
        }
        Ok(())
    }
}
