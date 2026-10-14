//! The provider registry: the adapters by API interface
//! (docs/architecture/extension-points.md, provider adapters).

use std::sync::Arc;

use mannie_types::provider::ApiInterface;

use crate::chat_completions::ChatCompletionsAdapter;
use crate::provider::Provider;
use crate::responses::ResponsesAdapter;
use crate::transport::HttpClient;

/// An adapter for an interface that already has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an adapter for {} is already registered", interface.as_str())]
pub struct DuplicateAdapter {
    /// The interface.
    pub interface: ApiInterface,
}

/// The registered adapters. A feature that supports another provider
/// registers its adapter here, in the composition root.
#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    adapters: Vec<Arc<dyn Provider>>,
}

impl ProviderRegistry {
    /// A registry without adapters.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with Mannie's adapters: Responses and Chat Completions.
    pub fn with_builtin(client: &HttpClient) -> Self {
        Self {
            adapters: vec![
                Arc::new(ResponsesAdapter::new(client.clone())),
                Arc::new(ChatCompletionsAdapter::new(client.clone())),
            ],
        }
    }

    /// Registers an adapter. Each interface has at most one.
    pub fn register(&mut self, adapter: Arc<dyn Provider>) -> Result<(), DuplicateAdapter> {
        let interface = adapter.interface();
        if self.adapter(interface).is_some() {
            return Err(DuplicateAdapter { interface });
        }
        self.adapters.push(adapter);
        Ok(())
    }

    /// The adapter for an interface.
    pub fn adapter(&self, interface: ApiInterface) -> Option<Arc<dyn Provider>> {
        self.adapters
            .iter()
            .find(|adapter| adapter.interface() == interface)
            .map(Arc::clone)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_types::provider::ApiInterface;

    use super::{DuplicateAdapter, ProviderRegistry};
    use crate::provider::{EventStream, ModelRequest, Provider, Target};

    #[derive(Debug)]
    struct FakeAdapter;

    impl Provider for FakeAdapter {
        fn interface(&self) -> ApiInterface {
            ApiInterface::Responses
        }

        fn stream(&self, _target: &Target, _request: ModelRequest) -> EventStream {
            Box::pin(futures_util::stream::empty())
        }
    }

    #[test]
    fn a_registered_adapter_is_found_by_its_interface_and_only_once() {
        let mut registry = ProviderRegistry::new();

        registry.register(Arc::new(FakeAdapter)).unwrap();
        let duplicate = registry.register(Arc::new(FakeAdapter));

        assert!(registry.adapter(ApiInterface::Responses).is_some());
        assert!(registry.adapter(ApiInterface::ChatCompletions).is_none());
        assert_eq!(
            duplicate,
            Err(DuplicateAdapter {
                interface: ApiInterface::Responses
            })
        );
    }
}
