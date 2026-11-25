//! Preview deployments and artifact retention.

use super::*;

impl SsrService {
    /// Starts a preview deployment without touching production.
    pub async fn activate_preview(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: &Path,
    ) -> Result<()> {
        validate_identifier("store_id", store_id)?;
        validate_identifier("deployment_id", deployment_id)?;
        let artifact_path = self.resolve_artifact_path(artifact_path).await?;
        let listen = free_loopback_port()?;
        let sequence = self.counter.fetch_add(1, Ordering::Relaxed);
        let group =
            RuntimeGroupId::new(format!("preview_{sequence}")).context("invalid preview group")?;
        let spec = DeploymentSpec {
            store_id: Some(store_id),
            deployment_id,
            artifact_path: &artifact_path,
            listen,
            preview: true,
            internal_token: None,
        };
        let limits = ArtifactLimits {
            max_bytes: self.config.artifact_max_bytes,
            max_files: self.config.artifact_max_files,
        };
        let mut manager = self.manager.lock().await;
        tracing::info!(store_id, deployment_id, path = %artifact_path.display(), "activating SSR preview");
        let deployment = start_deployment(
            &mut manager,
            &self.artifacts,
            limits,
            &self.config,
            &spec,
            group,
        )
        .await?;
        drop(manager);
        let deployment = Arc::new(deployment);
        self.retain_artifact(&deployment).await;
        let old = self
            .previews
            .write()
            .await
            .insert(store_id.to_owned(), deployment);
        if let Some(old) = old {
            let mut roots = self.asset_roots.write().await;
            if !roots.iter().any(|root| root == &old.asset_root) {
                roots.push(old.asset_root.clone());
            }
            drop(roots);
            let mut preview_roots = self.preview_asset_roots.write().await;
            if !preview_roots.iter().any(|root| root == &old.asset_root) {
                preview_roots.push(old.asset_root.clone());
            }
            drop(preview_roots);
            self.schedule_drain(old.clone());
        }
        self.refresh_deployment_metric().await;
        self.persist_state().await;
        Ok(())
    }
}

impl SsrService {
    /// Stops the store's preview deployment.
    pub async fn deactivate_preview(&self, store_id: &str) -> Result<()> {
        let Some(old) = self.previews.write().await.remove(store_id) else {
            return Ok(());
        };
        let mut roots = self.asset_roots.write().await;
        if !roots.iter().any(|root| root == &old.asset_root) {
            roots.push(old.asset_root.clone());
        }
        drop(roots);
        let mut preview_roots = self.preview_asset_roots.write().await;
        if !preview_roots.iter().any(|root| root == &old.asset_root) {
            preview_roots.push(old.asset_root.clone());
        }
        drop(preview_roots);
        self.schedule_drain(old.clone());
        self.refresh_deployment_metric().await;
        self.persist_state().await;
        Ok(())
    }
}

impl SsrService {
    pub(crate) async fn refresh_deployment_metric(&self) {
        let count = self.deployments.read().await.len() + self.previews.read().await.len();
        self.metrics.set_ssr_deployments(count);
    }
}

impl SsrService {
    pub(crate) async fn preview_deployment(&self, store_id: &str) -> Option<Arc<Deployment>> {
        self.previews.read().await.get(store_id).cloned()
    }
}

impl SsrService {
    pub(crate) async fn resolve_artifact_path(&self, path: &Path) -> Result<PathBuf> {
        let Some(root) = &self.artifact_root else {
            return Ok(path.to_owned());
        };
        let root = tokio::fs::canonicalize(root)
            .await
            .with_context(|| format!("failed to resolve artifact root {}", root.display()))?;
        let candidate = tokio::fs::canonicalize(path)
            .await
            .with_context(|| format!("failed to resolve artifact path {}", path.display()))?;
        if !candidate.starts_with(&root) {
            bail!("artifact path must stay under {}", root.display());
        }
        Ok(candidate)
    }
}

impl SsrService {
    pub(crate) async fn retain_artifact(&self, deployment: &Deployment) {
        *self
            .artifact_refs
            .write()
            .await
            .entry(deployment.artifact_dir.clone())
            .or_default() += 1;
    }
}

impl SsrService {
    pub(crate) fn schedule_drain(&self, deployment: Arc<Deployment>) {
        let manager = Arc::clone(&self.manager);
        let refs = Arc::clone(&self.artifact_refs);
        let roots = Arc::clone(&self.asset_roots);
        let preview_roots = Arc::clone(&self.preview_asset_roots);
        let artifact_dir = deployment.artifact_dir.clone();
        let group = deployment.group.clone();
        let delay = Duration::from_millis(self.config.drain_timeout_ms.max(1));
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // Same reasoning as in `replace`: tearing a group down mutates the
            // manager, and it happens after the drain delay so a request that
            // is still in flight is not cut off mid-response.
            match manager.lock().await.stop_group(&group).await {
                Ok(()) => {
                    release_artifact(artifact_dir, refs, roots, preview_roots).await;
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        group = %group,
                        "failed to drain SSR group; retaining its artifact"
                    );
                }
            }
        });
    }
}
