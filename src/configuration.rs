use serde::Deserialize;
use std::{path::PathBuf, str};
use tracing::*;

/// Use default config if no path is provided
pub fn load_configuration(config: Option<PathBuf>) -> Result<AppConfig, anyhow::Error> {
    let mut config_builder = config::Config::builder();

    if let Some(config) = config {
        info!("Using configuration from {:?}", config);
        config_builder = config_builder.add_source(config::File::with_name(
            config
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Failed to convert path"))?,
        ));
    } else {
        info!("Using dev configuration");
        config_builder = config_builder
            .add_source(config::File::with_name("configuration/settings"))
            .add_source(config::File::with_name("configuration/dev_settings"));
    }

    config_builder = config_builder.add_source(config::Environment::with_prefix("APP"));

    let config = config_builder.build()?;

    Ok(config.try_deserialize::<AppConfig>()?)
}

#[derive(Deserialize, Debug, Clone)]
pub struct AppConfig {
    pub mqtt: MqttConfig,
    pub zenoh: ZenohConfig,
}

// zenoh

#[derive(Deserialize, Debug, Clone)]
pub struct ZenohConfig {
    /// Endpoints to connect to
    #[serde(default)]
    pub connect: Vec<String>,
    /// Endpoints to listen on
    #[serde(default)]
    pub listen: Vec<String>,
    /// load zenoh configuration from file
    #[serde(default)]
    pub config_file_path: Option<String>,
    /// disable zenoh multicast scouting
    #[serde(default)]
    pub disable_multicast_scouting: bool,
    #[serde(default)]
    pub relayed_topics: Vec<ZenohTopic>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ZenohTopic {
    pub name: String,
    #[serde(default)]
    pub retained: bool,
}

// Mqtt

#[derive(Deserialize, Debug, Clone)]
pub struct MqttConfig {
    /// address of MQTT broker
    /// defaults to localhost
    #[serde(default = "default_mqtt_host")]
    pub address: String,
    /// network port of MQTT broker
    /// defaults to 1883
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    pub client_id: String,
    /// topics to subscribe to on MQTT broker
    /// defaults to `#` (all topics)
    #[serde(default = "default_mqtt_subscription")]
    pub subscriptions: Vec<String>,
    /// prefix to add to all outgoing mqtt topics
    pub mqtt_relay_prefix: Option<String>,
}

const DEFAULT_MQTT_PORT: u16 = 1883;

const fn default_mqtt_port() -> u16 {
    DEFAULT_MQTT_PORT
}

fn default_mqtt_host() -> String {
    "localhost".to_owned()
}

const MQTT_WILDCARD_TOPIC: &str = "#";

fn default_mqtt_subscription() -> Vec<String> {
    vec![MQTT_WILDCARD_TOPIC.to_owned()]
}
