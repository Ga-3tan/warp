use std::cell::Cell;
use std::rc::Rc;

use ai::api_keys::CustomEndpointSchema;
use settings::Setting as _;
use warpui::{App, SingletonEntity as _};
use warpui_extras::secure_storage;

use super::*;
use crate::settings::init_and_register_user_preferences;
#[test]
fn endpoint_edits_preserve_toolsets_by_stable_model_key() {
    let saved = CustomEndpointDefinition {
        name: "Gateway".into(),
        base_url: "https://gateway.example/v1".into(),
        schema: CustomEndpointSchema::AnthropicMessages,
        models: vec![CustomEndpointModel {
            name: "model".into(),
            config_key: "pinned-key".into(),
            computer_use_toolset: ai::api_keys::ComputerUseToolset::AnthropicComputer20251124,
            ..Default::default()
        }],
    };
    let edited = definition_from_params(
        CustomEndpointParams {
            name: "Renamed gateway".into(),
            url: saved.base_url.clone(),
            api_key: "rotated-key".into(),
            schema: saved.schema,
            models: vec![
                (
                    "model".into(),
                    Some("Alias".into()),
                    Some("pinned-key".into()),
                ),
                ("new".into(), None, None),
            ],
        },
        Some(&saved),
    );
    assert_eq!(
        edited.models[0].computer_use_toolset,
        saved.models[0].computer_use_toolset
    );
    assert_eq!(
        edited.models[1].computer_use_toolset,
        ai::api_keys::ComputerUseToolset::None
    );
}

struct FailingSecureStorage {
    write_attempts: Rc<Cell<usize>>,
}

impl secure_storage::SecureStorage for FailingSecureStorage {
    fn write_value(&self, _key: &str, _value: &str) -> Result<(), secure_storage::Error> {
        self.write_attempts.set(self.write_attempts.get() + 1);
        Err(secure_storage::Error::Unknown(anyhow::anyhow!(
            "deliberate test failure"
        )))
    }

    fn read_value(&self, _key: &str) -> Result<String, secure_storage::Error> {
        Err(secure_storage::Error::NotFound)
    }

    fn remove_value(&self, _key: &str) -> Result<(), secure_storage::Error> {
        Ok(())
    }
}

#[test]
fn removal_succeeds_when_credential_cleanup_fails() {
    App::test((), |mut app| async move {
        app.update(init_and_register_user_preferences);
        app.add_singleton_model(AISettings::new_with_defaults);

        let write_attempts = Rc::new(Cell::new(0));
        let storage_write_attempts = write_attempts.clone();
        app.add_singleton_model(move |_| -> secure_storage::Model {
            Box::new(FailingSecureStorage {
                write_attempts: storage_write_attempts,
            })
        });
        app.add_singleton_model(ApiKeyManager::new);

        let mut definitions = CustomEndpointDefinitions::default();
        definitions
            .insert(
                CustomEndpointId::generated(),
                CustomEndpointDefinition {
                    name: "Test".to_owned(),
                    base_url: "https://example.com/v1".to_owned(),
                    schema: CustomEndpointSchema::default(),
                    models: vec![CustomEndpointModel {
                        name: "model".to_owned(),
                        alias: None,
                        config_key: "config-key".to_owned(),
                        computer_use_toolset: Default::default(),
                    }],
                },
            )
            .unwrap();
        AISettings::handle(&app)
            .update(&mut app, |settings, ctx| {
                settings
                    .custom_endpoints
                    .load_value(definitions.clone(), true, ctx)
            })
            .unwrap();
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_custom_endpoint_definitions(definitions, ctx);
        });

        assert!(app.update(|ctx| remove(0, ctx)).is_ok());
        assert_eq!(write_attempts.get(), 1);
        AISettings::handle(&app).read(&app, |settings, _| {
            assert!(settings.custom_endpoints.value().is_empty());
        });
        ApiKeyManager::handle(&app).read(&app, |manager, _| {
            assert!(
                manager
                    .custom_endpoint_definitions()
                    .is_some_and(CustomEndpointDefinitions::is_empty)
            );
            assert!(manager.custom_endpoints().is_empty());
        });
    });
}
