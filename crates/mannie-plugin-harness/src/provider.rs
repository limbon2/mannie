//! The provider's parts that plugin tests read and script: the model's
//! requests and their content, the events of a reply, the steps of the fake
//! model and a fixture server of a provider's API.

pub use mannie_provider::testing::{FixtureResponse, FixtureServer, Step};
pub use mannie_provider::{Content, ModelRequest, ProviderEvent, Role};
