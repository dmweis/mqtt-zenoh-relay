mod configuration;

use anyhow::Context;
use clap::Parser;
use configuration::ZenohTopic;
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, Outgoing, QoS};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{dispatcher, error, info, Dispatch};
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, EnvFilter, Registry};
use zenoh::config::Config;
use zenoh::prelude::r#async::*;

const MQTT_MAX_PACKET_SIZE: usize = 268435455;

/// technically speaking we want to use AtLeastOnce
/// but honestly we don't care about message delivery here
const MQTT_LOGGER_QOS: QoS = QoS::AtMostOnce;

#[derive(Parser, Debug)]
#[command()]
struct Args {
    /// configuration path
    #[clap(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    setup_tracing()?;
    let config = configuration::load_configuration(args.config)?;

    // Configure MQTT
    let mut mqtt_options = MqttOptions::new(
        &config.mqtt.client_id,
        &config.mqtt.address,
        config.mqtt.port,
    );
    mqtt_options.set_keep_alive(Duration::from_secs(5));
    // outgoing is default
    mqtt_options.set_max_packet_size(MQTT_MAX_PACKET_SIZE, 10 * 1024);
    info!(?mqtt_options, "Starting MQTT server with options",);

    let (mqtt_client, mut event_loop) = AsyncClient::new(mqtt_options, 10);

    // MQTT subscriptions
    for subscription in &config.mqtt.subscriptions {
        mqtt_client.subscribe(subscription, MQTT_LOGGER_QOS).await?;
    }

    // configure zenoh
    let mut zenoh_config = if let Some(config_file) = config.zenoh.config_file_path {
        Config::from_file(config_file).map_err(MqttZenohRelayError::ZenohError)?
    } else {
        Config::default()
    };

    if !config.zenoh.connect.is_empty() {
        zenoh_config.connect.endpoints = config
            .zenoh
            .connect
            .iter()
            .map(|v| v.parse().map_err(MqttZenohRelayError::ZenohError))
            .collect::<MqttZenohRelayResult<Vec<_>>>()?;
    }
    if !config.zenoh.listen.is_empty() {
        zenoh_config.listen.endpoints = config
            .zenoh
            .listen
            .iter()
            .map(|v| v.parse().map_err(MqttZenohRelayError::ZenohError))
            .collect::<MqttZenohRelayResult<Vec<_>>>()?;
    }
    // this one is odd?
    if config.zenoh.disable_multicast_scouting {
        zenoh_config
            .scouting
            .multicast
            .set_enabled(Some(config.zenoh.disable_multicast_scouting))
            .map_err(MqttZenohRelayError::FailedToSetZenohMulticastScouting)?;
    }

    let zenoh_session = zenoh::open(zenoh_config)
        .res()
        .await
        .map_err(MqttZenohRelayError::ZenohError)?;
    let zenoh_session = zenoh_session.into_arc();

    // zenoh subscriptions
    for subscription in &config.zenoh.relayed_topics {
        info!(?subscription, "Subscribing to zenoh topic");
        let subscriber = zenoh_session
            .declare_subscriber(&subscription.name)
            .res()
            .await
            .map_err(MqttZenohRelayError::ZenohError)?;
        tokio::spawn({
            let mqtt_client = mqtt_client.clone();
            let subscription = subscription.clone();
            async move {
                loop {
                    if let Err(err) =
                        zenoh_subscribe_loop(&mqtt_client, &subscriber, &subscription).await
                    {
                        error!(error = %err, "Error in zenoh subscribe loop");
                    }
                }
            }
        });
    }

    // we could just use `Session::put` for publishing but I like having explicit publishers
    let mut publisher_table: HashMap<String, zenoh::publication::Publisher<'_>> = HashMap::new();

    let join = tokio::spawn(async move {
        loop {
            match mqtt_receive_loop(
                &mut event_loop,
                &mqtt_client,
                &zenoh_session,
                config.mqtt.subscriptions.clone(),
                &mut publisher_table,
                &config.mqtt.mqtt_relay_prefix,
            )
            .await
            {
                Ok(_) => {
                    info!("MQTT receive loop exited");
                    break;
                }
                Err(e) => {
                    error!(error = %e, "Error in MQTT receive loop");
                }
            }
        }
    });

    join.await?;

    Ok(())
}

/// Mqtt receive loop
async fn mqtt_receive_loop(
    event_loop: &mut EventLoop,
    mqtt_client: &AsyncClient,
    zenoh_session: &Arc<Session>,
    mqtt_subscriptions: Vec<String>,
    zenoh_publisher_table: &mut HashMap<String, zenoh::publication::Publisher<'_>>,
    mqtt_relay_prefix: &Option<String>,
) -> anyhow::Result<()> {
    loop {
        match event_loop.poll().await {
            Ok(notification) => match notification {
                Event::Incoming(Incoming::Publish(publish)) => {
                    let zenoh_topic = match mqtt_relay_prefix {
                        Some(ref prefix) => format!("{}/{}", prefix, publish.topic),
                        None => publish.topic.clone(),
                    };

                    let value: Value = publish.payload.to_vec().into();
                    let value = value.encoding(Encoding::Exact(KnownEncoding::TextPlain));
                    if let Some(publisher) = zenoh_publisher_table.get(&zenoh_topic) {
                        // reuse publisher
                        publisher
                            .put(value)
                            .res()
                            .await
                            .map_err(MqttZenohRelayError::ZenohError)?;
                    } else {
                        // create new publisher for fresh topic
                        info!(mqtt_topic = ?publish.topic, zenoh_topic, "Creating new publisher for topic");
                        let publisher = zenoh_session
                            .declare_publisher(zenoh_topic.clone())
                            .res()
                            .await
                            .map_err(MqttZenohRelayError::ZenohError)?;
                        publisher
                            .put(value)
                            .res()
                            .await
                            .map_err(MqttZenohRelayError::ZenohError)?;
                        zenoh_publisher_table.insert(zenoh_topic, publisher);
                    }
                }
                Event::Incoming(Incoming::ConnAck(_)) => {
                    info!("Connected to MQTT broker. Subscribing to all topics");
                    for subscription in &mqtt_subscriptions {
                        mqtt_client.subscribe(subscription, MQTT_LOGGER_QOS).await?;
                    }
                }
                Event::Outgoing(Outgoing::Disconnect) => {
                    info!("Client disconnected. Shutting down");
                    break;
                }
                _ => (),
            },
            Err(e) => {
                error!(error = %e, "Error processing event loop notifications");
            }
        }
    }
    Ok(())
}

pub fn setup_tracing() -> anyhow::Result<()> {
    let filter = EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
        .parse("")?;

    let subscriber = Registry::default()
        .with(filter)
        .with(tracing_logfmt::layer());
    dispatcher::set_global_default(Dispatch::new(subscriber))
        .context("Global logger has already been set!")?;
    Ok(())
}

type MqttZenohRelayResult<T> = Result<T, MqttZenohRelayError>;

#[derive(Error, Debug)]
pub enum MqttZenohRelayError {
    #[error("Zenoh error")]
    ZenohError(#[from] zenoh::Error),
    #[error("Failed to set zenoh multicast scouting {0:?}")]
    FailedToSetZenohMulticastScouting(Option<bool>),
}

async fn zenoh_subscribe_loop(
    mqtt_client: &AsyncClient,
    zenoh_subscriber: &zenoh::subscriber::Subscriber<'_, flume::Receiver<Sample>>,
    topic: &ZenohTopic,
) -> anyhow::Result<()> {
    loop {
        let sample = zenoh_subscriber.recv_async().await?;
        let payload: Vec<u8> = sample.value.try_into()?;
        mqtt_client
            .publish(sample.key_expr, MQTT_LOGGER_QOS, topic.retained, payload)
            .await?;
    }
}
