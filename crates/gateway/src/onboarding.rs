//! Gateway adapter: wraps `LiveOnboardingService` to implement `OnboardingService`.

use {async_trait::async_trait, serde_json::Value};

use crate::services::{OnboardingService, ServiceResult};

/// Gateway-side onboarding service backed by `moltis_onboarding::service::LiveOnboardingService`.
pub struct GatewayOnboardingService {
    inner: moltis_onboarding::service::LiveOnboardingService,
}

impl GatewayOnboardingService {
    pub fn new(inner: moltis_onboarding::service::LiveOnboardingService) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl OnboardingService for GatewayOnboardingService {
    async fn wizard_start(&self, params: Value) -> ServiceResult {
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(self.inner.wizard_start(force))
    }

    async fn wizard_next(&self, params: Value) -> ServiceResult {
        let input = params.get("input").and_then(|v| v.as_str()).unwrap_or("");
        self.inner.wizard_next(input)
    }

    async fn wizard_cancel(&self) -> ServiceResult {
        self.inner.wizard_cancel();
        Ok(serde_json::json!({}))
    }

    async fn wizard_status(&self) -> ServiceResult {
        Ok(self.inner.wizard_status())
    }

    async fn identity_get(&self) -> ServiceResult {
        Ok(serde_json::to_value(self.inner.identity_get()).unwrap_or_default())
    }

    async fn identity_update(&self, params: Value) -> ServiceResult {
        self.inner
            .identity_update(params)
            .map_err(|e| e.to_string())
    }

    async fn identity_update_soul(&self, soul: Option<String>) -> ServiceResult {
        self.inner
            .identity_update_soul(soul)
            .map_err(|e| e.to_string())
    }

    #[cfg(feature = "agent")]
    async fn identity_get_for_agent(&self, agent_id: &str) -> ServiceResult {
        Ok(serde_json::to_value(self.inner.identity_get_for_agent(agent_id)).unwrap_or_default())
    }

    #[cfg(feature = "agent")]
    async fn identity_update_for_agent(
        &self,
        agent_id: &str,
        params: serde_json::Value,
    ) -> ServiceResult {
        self.inner
            .identity_update_for_agent(agent_id, params)
            .map_err(|e| e.to_string())
    }

    #[cfg(feature = "agent")]
    async fn identity_update_soul_for_agent(
        &self,
        agent_id: &str,
        soul: Option<String>,
    ) -> ServiceResult {
        self.inner
            .identity_update_soul_for_agent(agent_id, soul)
            .map_err(|e| e.to_string())
    }
}
