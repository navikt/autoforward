use serde::Deserialize;

#[derive(Clone, Deserialize, Debug)]
pub struct KubernetesResponse {
    pub items: Vec<ApplicationResource>,
}

#[derive(Clone, Deserialize, Debug)]
pub struct ResourceMetadata {
    pub name: String,
}

#[derive(Clone, Deserialize, Debug)]
pub struct ApplicationResource {
    pub spec: ApplicationResourceSpec,
    pub metadata: ResourceMetadata,
}

#[derive(Clone, Deserialize, Debug)]
pub struct ApplicationResourceSpec {
    pub ingresses: Option<Vec<String>>,
    pub liveness: Option<HealthCheck>,
    pub readiness: Option<HealthCheck>,
}

#[derive(Clone, Deserialize, Debug)]
pub struct HealthCheck {
    pub path: String,
}
