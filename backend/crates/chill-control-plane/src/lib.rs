//! Tenant-safe control-plane domain and persistence boundaries.

mod access;
mod analytics;
mod bootstrap;
mod collection;
mod console;
mod console_http;
mod credential;
mod model;
mod privacy;
mod sites_auth;
mod store;

pub use access::{
    AuthenticatedServiceCredential, IssuedServiceCredential, IssuedUserSession,
    ServiceCredentialRequest, VerifiedIdentity,
};
pub use analytics::{
    Alert, AnalyticsScope, AnalyticsWorkspace, CreateAlertRequest, CreateDashboardRequest,
    CreateSavedQueryRequest, Dashboard, DebuggerSnapshot, SavedQuery,
};
pub use bootstrap::{
    BootstrapDataSource, BootstrapEnvironment, BootstrapOwner, BootstrapPrivacyPolicy,
    BootstrapQuota, BootstrapRequest, BootstrapResult, BootstrapSDKKey, BootstrapSamplingPolicy,
    BootstrapSchema, NamedSlug,
};
pub use collection::{CollectionPolicy, SetCollectionPolicyRequest, collection_router};
pub use console::{
    ActivatePrivacyRequest, ActivateSamplingRequest, ConsoleActor, ConsoleDataSource,
    ConsoleEnvironment, ConsoleOrganization, ConsoleOverview, ConsolePrivacyPolicy, ConsoleProject,
    ConsoleSDKKey, ConsoleSamplingPolicy, ConsoleSchema, CreateDataSourceRequest,
    CreateEnvironmentRequest, CreateProjectRequest, CreateSDKKeyRequest, CreateSchemaRequest,
    IssuedSDKKey, RotatedSDKKey,
};
pub use console_http::console_router;
pub use credential::{
    Credential, CredentialError, CredentialIssuer, CredentialKind, parse_credential_prefix,
};
pub use model::{AuthenticatedSDKKey, AuthenticatedUserSession, Capability, Role};
pub use privacy::{
    EnvironmentPrivacyPolicy, annotation_classification_is_eligible,
    parse_environment_privacy_policy,
};
pub use sites_auth::{SitesAuthentication, sites_auth_router};
pub use store::{ControlPlaneError, Store};
