# Mqtt Zenoh Relay

[![Rust](https://github.com/dmweis/mqtt-zenoh-relay/workflows/Rust/badge.svg)](https://github.com/dmweis/mqtt-zenoh-relay/actions)
[![Private docs](https://github.com/dmweis/mqtt-zenoh-relay/workflows/Deploy%20Docs%20to%20GitHub%20Pages/badge.svg)](https://davidweis.dev/mqtt-zenoh-relay/mqtt_zenoh_relay/index.html)

## Install deb

Install with `make install`  

Edit config file at `/etc/mqtt_zenoh_relay/settings.yaml`  

### Systemd service

`mqtt-zenoh-relay`

```bash
sudo systemctl restart mqtt-zenoh-relay

sudo journalctl -u mqtt-zenoh-relay -f
```
