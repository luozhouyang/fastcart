//! Deployment lifecycle and shutdown.

use super::*;

impl SsrService {
    /// Imports configured artifacts and starts one workerd group per deployment.
    pub async fn initialize(
        config: &SsrConfig,
        metrics: Arc<Metrics>,
        credentials: CredentialManager,
    ) -> Result<Option<Arc<Self>>> {
        if !config.enabled {
            return Ok(None);
        }
        let artifacts = ArtifactStore::new(config.runtime_dir.join("artifacts"));
        let limits = ArtifactLimits {
            max_bytes: config.artifact_max_bytes,
            max_files: config.artifact_max_files,
        };
        let mut manager_config =
            RuntimeManagerConfig::new(config.runtime_dir.join("runtime"), config.binary.clone());
        manager_config.startup_timeout = Duration::from_millis(config.startup_timeout_ms);
        manager_config.shutdown_timeout = Duration::from_millis(config.shutdown_timeout_ms);
        manager_config.request_timeout = Duration::from_millis(config.request_timeout_ms);
        manager_config.restart_backoff = Duration::from_millis(config.restart_backoff_ms);
        manager_config.max_restarts = config.max_restarts;
        let mut manager = RuntimeManager::new(manager_config)
            .context("failed to create the SSR runtime manager")?;
        let state_path = config.runtime_dir.join("deployment-state.json");
        let persisted_config = if config.deployments.is_empty() {
            load_persisted(&state_path).await.map(|entries| {
                let mut persisted = config.clone();
                persisted.deployments = entries
                    .into_iter()
                    .map(|entry| crate::config::SsrDeploymentConfig {
                        store_id: entry.store_id,
                        deployment_id: entry.deployment_id,
                        artifact_path: entry.artifact_path,
                        listen: None,
                        preview: entry.preview,
                        internal_token: None,
                    })
                    .collect();
                persisted
            })
        } else {
            None
        };
        let specs = deployment_specs(persisted_config.as_ref().unwrap_or(config));
        let mut deployments = HashMap::new();
        let mut previews = HashMap::new();
        let mut default = None;
        let mut started = Vec::new();
        for spec in specs {
            let group = RuntimeGroupId::new(spec.store_id.unwrap_or("default"))
                .context("invalid SSR runtime group")?;
            let result = start_deployment(
                &mut manager,
                &artifacts,
                limits,
                config,
                &spec,
                group.clone(),
            )
            .await;
            match result {
                Ok(deployment) => {
                    started.push(group);
                    if spec.store_id.is_some() {
                        let deployment = Arc::new(deployment);
                        if spec.preview {
                            previews
                                .insert(spec.store_id.unwrap_or_default().to_owned(), deployment);
                        } else {
                            deployments
                                .insert(spec.store_id.unwrap_or_default().to_owned(), deployment);
                        }
                    } else {
                        default = Some(Arc::new(deployment));
                    }
                }
                Err(error) => {
                    for group in &started {
                        if let Err(stop_error) = manager.stop_group(group).await {
                            tracing::warn!(error = %stop_error, group = %group, "failed to stop partially started SSR group");
                        }
                    }
                    return Err(error);
                }
            }
        }
        let mut asset_roots: Vec<PathBuf> = deployments
            .values()
            .chain(previews.values())
            .map(|deployment| deployment.asset_root.clone())
            .collect();
        if let Some(default) = &default {
            asset_roots.push(default.asset_root.clone());
        }
        let preview_asset_roots: Vec<PathBuf> = previews
            .values()
            .map(|deployment| deployment.asset_root.clone())
            .collect();
        let mut artifact_refs: HashMap<PathBuf, usize> = HashMap::new();
        for deployment in deployments
            .values()
            .chain(previews.values())
            .chain(default.iter())
        {
            *artifact_refs
                .entry(deployment.artifact_dir.clone())
                .or_default() += 1;
        }
        let active_artifacts: HashSet<PathBuf> = artifact_refs.keys().cloned().collect();
        sweep_orphan_artifacts(artifacts.root(), &active_artifacts).await;
        metrics.set_ssr_deployments(deployments.len() + previews.len());
        Ok(Some(Arc::new(Self {
            manager: Arc::new(Mutex::new(manager)),
            deployments: RwLock::new(deployments),
            previews: RwLock::new(previews),
            history: RwLock::new(HashMap::new()),
            asset_roots: Arc::new(RwLock::new(asset_roots)),
            preview_asset_roots: Arc::new(RwLock::new(preview_asset_roots)),
            artifact_refs: Arc::new(RwLock::new(artifact_refs)),
            response_cache: RwLock::new(HashMap::new()),
            default,
            metrics,
            credentials,
            artifacts,
            artifact_root: config.artifact_root.clone(),
            state_path,
            config: config.clone(),
            counter: AtomicU64::new(0),
        })))
    }
}

impl SsrService {
    /// Starts a new artifact for one store and drains the replaced deployment.
    pub async fn activate(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: &Path,
    ) -> Result<()> {
        self.replace(store_id, deployment_id, artifact_path, true)
            .await
    }
}

impl SsrService {
    /// Restores the deployment replaced by the last activation.
    pub async fn rollback(&self, store_id: &str) -> Result<()> {
        let previous = {
            let mut history = self.history.write().await;
            history.get_mut(store_id).and_then(std::vec::Vec::pop)
        };
        let Some(previous) = previous else {
            bail!("no previous deployment for {store_id}");
        };
        self.replace(
            store_id,
            &previous.deployment_id,
            &previous.artifact_path,
            true,
        )
        .await
    }
}

impl SsrService {
    async fn replace(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: &Path,
        record_history: bool,
    ) -> Result<()> {
        validate_identifier("store_id", store_id)?;
        validate_identifier("deployment_id", deployment_id)?;
        let artifact_path = self.resolve_artifact_path(artifact_path).await?;
        let listen = free_loopback_port()?;
        let sequence = self.counter.fetch_add(1, Ordering::Relaxed);
        let group = RuntimeGroupId::new(format!("active_{sequence}"))
            .context("invalid activation group")?;
        let spec = DeploymentSpec {
            store_id: Some(store_id),
            deployment_id,
            artifact_path: &artifact_path,
            listen,
            preview: false,
            internal_token: self.config.internal_token.clone(),
        };
        let limits = ArtifactLimits {
            max_bytes: self.config.artifact_max_bytes,
            max_files: self.config.artifact_max_files,
        };
        // The manager lock is held across the await on purpose, and it is the
        // one guard in this service that is not merely protecting a map.
        // `start_deployment` needs `&mut RuntimeManager` because the manager is
        // the state it mutates — there is no inner mutability to borrow
        // instead — and spinning up a workerd group is not something two
        // deployments should interleave. Serving requests does not take this
        // lock: it reads `deployments`/`previews` under their own RwLock, so the
        // only thing serialized here is deployment against deployment.
        let mut manager = self.manager.lock().await;
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
            .deployments
            .write()
            .await
            .insert(store_id.to_owned(), deployment);
        let previous = old.or_else(|| self.default.clone());
        if record_history && let Some(previous) = &previous {
            let mut history = self.history.write().await;
            let entries = history.entry(store_id.to_owned()).or_default();
            entries.push(DeploymentMeta {
                deployment_id: previous.deployment_id.clone(),
                artifact_path: previous.artifact_path.clone(),
            });
            if entries.len() > 3 {
                entries.remove(0);
            }
        }
        self.invalidate_store_cache(store_id).await;
        self.refresh_deployment_metric().await;
        self.persist_state().await;
        if let Some(previous) = previous {
            tracing::info!(store_id, previous = %previous.deployment_id, "draining replaced SSR deployment");
            let mut roots = self.asset_roots.write().await;
            if !roots.iter().any(|root| root == &previous.asset_root) {
                roots.push(previous.asset_root.clone());
            }
            drop(roots);
            if self
                .default
                .as_ref()
                .is_none_or(|default| default.group.as_str() != previous.group.as_str())
            {
                self.schedule_drain(previous.clone());
            }
        }
        Ok(())
    }
}

impl SsrService {
    pub(crate) async fn persist_state(&self) {
        let mut entries: Vec<PersistedDeployment> = self
            .deployments
            .read()
            .await
            .iter()
            .map(|(store_id, deployment)| PersistedDeployment {
                store_id: store_id.clone(),
                deployment_id: deployment.deployment_id.clone(),
                artifact_path: deployment.artifact_path.clone(),
                preview: false,
            })
            .collect();
        entries.extend(
            self.previews
                .read()
                .await
                .iter()
                .map(|(store_id, deployment)| PersistedDeployment {
                    store_id: store_id.clone(),
                    deployment_id: deployment.deployment_id.clone(),
                    artifact_path: deployment.artifact_path.clone(),
                    preview: true,
                }),
        );
        if entries.is_empty() {
            return;
        }
        let Ok(serialized) = serde_json::to_vec_pretty(&entries) else {
            tracing::warn!("failed to serialize SSR deployment state");
            return;
        };
        let staging = self.state_path.with_extension("json.tmp");
        if let Some(parent) = self.state_path.parent()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            tracing::warn!(error = %error, "failed to create SSR state directory");
            return;
        }
        if let Err(error) = tokio::fs::write(&staging, serialized).await {
            tracing::warn!(error = %error, "failed to write SSR deployment state");
            return;
        }
        if let Err(error) = tokio::fs::rename(&staging, &self.state_path).await {
            tracing::warn!(error = %error, "failed to commit SSR deployment state");
        }
    }
}

impl SsrService {
    /// Stops every runtime group during server shutdown.
    pub async fn shutdown(&self) {
        let mut manager = self.manager.lock().await;
        let mut groups: Vec<RuntimeGroupId> = self
            .deployments
            .read()
            .await
            .values()
            .chain(self.previews.read().await.values())
            .map(|deployment| deployment.group.clone())
            .collect();
        if let Some(default) = &self.default {
            groups.push(default.group.clone());
        }
        for group in groups {
            if let Err(error) = manager.stop_group(&group).await {
                tracing::warn!(error = %error, group = %group, "failed to stop the universal SSR runtime");
            }
        }
        self.metrics.set_ssr_deployments(0);
    }
}
