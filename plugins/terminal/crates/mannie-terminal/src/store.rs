//! The plugin's table, `terminal_settings` (plugins/terminal/docs/README.md,
//! data): one row with the settings. Nothing of a terminal is stored: its
//! output and the owner's input live in memory only, and what reaches the
//! model is stored by the core with the tool's results and the mentions.

use std::sync::Arc;

use mannie_plugin_sdk::services::store::{
    Durability, Expect, Migration, ModuleStoreApi, Row, Statement, StoreError, StoreModule,
    WriteBatch,
};

use crate::commands::{Renderer, TerminalSettings};

/// The prefix of the plugin's tables, which names its module of the store.
pub(crate) const PREFIX: &str = "terminal";

const MIGRATIONS: &[Migration] = &[Migration::new(
    1,
    "create the table of the settings",
    "CREATE TABLE terminal_settings (
        id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
        default_shell TEXT,
        scrollback INTEGER NOT NULL CHECK (scrollback BETWEEN 500 AND 20000),
        font_size INTEGER NOT NULL CHECK (font_size BETWEEN 10 AND 24),
        cursor_blink INTEGER NOT NULL CHECK (cursor_blink IN (0, 1)),
        renderer TEXT NOT NULL CHECK (renderer IN ('automatic', 'gpu', 'compatible')),
        screen_reader INTEGER NOT NULL CHECK (screen_reader IN (0, 1))
    ) STRICT",
)];

const SELECT_SETTINGS: &str = "SELECT default_shell, scrollback, font_size, cursor_blink, \
    renderer, screen_reader FROM terminal_settings WHERE id = 1";

const UPSERT_SETTINGS: &str = "INSERT INTO terminal_settings (id, default_shell, scrollback, \
    font_size, cursor_blink, renderer, screen_reader) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6) \
    ON CONFLICT (id) DO UPDATE SET default_shell = excluded.default_shell, \
    scrollback = excluded.scrollback, font_size = excluded.font_size, \
    cursor_blink = excluded.cursor_blink, renderer = excluded.renderer, \
    screen_reader = excluded.screen_reader";

/// The plugin's module of the store.
pub const MODULE: StoreModule =
    StoreModule::new(PREFIX, MIGRATIONS, &[SELECT_SETTINGS, UPSERT_SETTINGS]);

/// The renderer as the table keeps it.
fn renderer_text(renderer: Renderer) -> &'static str {
    match renderer {
        Renderer::Automatic => "automatic",
        Renderer::Gpu => "gpu",
        Renderer::Compatible => "compatible",
    }
}

/// The settings of a row.
fn settings(row: &Row) -> Result<TerminalSettings, StoreError> {
    let renderer: String = row.get(4)?;
    Ok(TerminalSettings {
        default_shell: row.get(0)?,
        scrollback: row.get(1)?,
        font_size: row.get(2)?,
        cursor_blink: row.get(3)?,
        renderer: match renderer.as_str() {
            "gpu" => Renderer::Gpu,
            "compatible" => Renderer::Compatible,
            _ => Renderer::Automatic,
        },
        screen_reader: row.get(5)?,
    })
}

/// The plugin's table, through its module of the store.
#[derive(Debug, Clone)]
pub(crate) struct SettingsStore {
    store: Arc<dyn ModuleStoreApi>,
}

impl SettingsStore {
    pub(crate) fn new(store: Arc<dyn ModuleStoreApi>) -> Self {
        Self { store }
    }

    /// The settings, or the defaults before the owner changed any.
    pub(crate) async fn read(&self) -> Result<TerminalSettings, StoreError> {
        let rows = self.store.query(Statement::new(SELECT_SETTINGS)).await?;
        rows.first()
            .map_or_else(|| Ok(TerminalSettings::default()), settings)
    }

    /// Stores the settings.
    pub(crate) async fn write(&self, value: &TerminalSettings) -> Result<(), StoreError> {
        let upsert = Statement::new(UPSERT_SETTINGS)
            .bind(value.default_shell.clone())
            .bind(value.scrollback)
            .bind(value.font_size)
            .bind(value.cursor_blink)
            .bind(renderer_text(value.renderer))
            .bind(value.screen_reader);
        self.store
            .write(
                WriteBatch::new().push(upsert, Expect::Changes(1)),
                Durability::Full,
            )
            .await
            .map(drop)
    }
}
