# Deploying vitrine as a real kiosk

How a device boots straight into the kiosk with no desktop, no login
prompt, and automatic recovery at every layer.

## The layering

```
systemd ──starts──▶ vitrine (session, GPU, input)
                       └─watchdog──▶ the kiosk app
```

Two supervision layers: systemd restarts *vitrine* if the compositor itself
dies; vitrine's watchdog restarts *the app*. Every failure short of a kernel
panic self-heals.

## 1. Install

```bash
cargo build --release
sudo install -Dm755 target/release/vitrine /usr/local/bin/vitrine
sudo install -Dm644 vitrine.toml /etc/vitrine/vitrine.toml
```

## 2. A dedicated user

Never run a kiosk as your own account, and never as root — libseat grants
device access to plain users:

```bash
sudo useradd -m -G video,input kiosk
```

## 3. systemd unit

`/etc/systemd/system/vitrine.service`:

```ini
[Unit]
Description=vitrine kiosk compositor
# Take over the VT that getty would otherwise own
Conflicts=getty@tty1.service
After=systemd-user-sessions.service

[Service]
User=kiosk
# A logind session bound to a VT is what lets libseat hand us the GPU
PAMName=login
TTYPath=/dev/tty1
StandardInput=tty
StandardOutput=journal
StandardError=journal
ExecStart=/usr/local/bin/vitrine --tty --config /etc/vitrine/vitrine.toml
# Layer-1 supervision: if the compositor dies, systemd brings it back
Restart=always
RestartSec=2

[Install]
WantedBy=graphical.target
```

```bash
sudo systemctl enable vitrine
sudo systemctl set-default graphical.target
```

Reboot: the machine comes up showing the configured app fullscreen. Journald
captures the frame stats and watchdog events (`journalctl -u vitrine`).

## Notes

- **Ubuntu Frame comparison**: Frame ships this arrangement as a snap with
  the daemon plumbing prepackaged; the unit above is the same idea by hand.
- **Recovery test**: `sudo pkill -x vitrine` (systemd restarts the
  compositor), then `pkill` the app (vitrine's watchdog restarts it).
- **Escape hatch on a dev box**: Ctrl+Alt+F2 still VT-switches (vitrine
  forwards `XF86Switch_VT_n` to the session); disable that in a hardened
  deployment by dropping the keybinding.
