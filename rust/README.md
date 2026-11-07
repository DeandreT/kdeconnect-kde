# Rust desktop architecture

The target is a Linux-first Rust-native desktop application and daemon compatible
with the existing Android protocol. The original implementation remains available
throughout the port. Rust code is GPL-3.0-only; original files retain their notices.

## Boundaries

- `protocol`: bounded, typed wire envelopes, independent of desktop services.
- `sms`: message validation and owned image attachments, independent of the GUI.
- `desktop`: egui interface and a background D-Bus compatibility adapter.

The compatibility adapter is transitional. It does not replace pairing, TLS,
device trust, or the network service. No connection is trusted based on a device
name or discovery packet alone. The native transport must enforce explicit
pairing, certificate pinning, and bounded untrusted input before plugins run.

The SMS composer reads RGBA clipboard images, encodes them as PNG, previews them,
and passes private temporary file paths to the existing synchronous `sendSms`
method. The draft and background send job share attachment ownership, so files
remain available throughout the call. A failed call retains the draft and does
not retry automatically: an uncertain result must not duplicate a message.
D-Bus acceptance is not a delivery receipt. New messages use the phone's default
SIM until conversation-specific SIM selection is implemented.

## Build and run

Use an external build directory on a disk with sufficient space:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
dbus-run-session -- env KDECONNECT_RS_TEST_BUS=1 cargo test -p kdeconnect-rs mock_service -- --ignored
cargo build --workspace
cargo run -p kdeconnect-rs -- --demo
cargo run -p kdeconnect-rs
```

`--demo` never connects to D-Bus or sends messages. Normal mode requires the
installed KDE Connect service, a paired reachable phone, and SMS permission.
Refresh the device list, select a phone, enter a recipient, then paste an image
into the focused message box with Ctrl+V or use **Paste image**. Plain-text paste
continues to use the editor. Oversize images are rejected rather than silently
resized. The combined draft limit is 600,000 bytes, matching the original composer;
carrier limits may differ. Sending requires an explicit confirmation.

To exercise real keyboard and image clipboard behavior without touching your
desktop session, run `python rust/desktop/tests/gui_smoke.py <built-binary>
<output-directory>`. This requires Xvfb, xdotool, xclip, ImageMagick, and Tesseract.
The script assigns a private X11 display, launches demo mode, checks text/image
paste, image copy, cancellation and simulated sending, then closes its processes.

## Manual phone verification

First use demo mode to check text paste, image paste, the preview's **Copy image**
button, and cancellation. Then run normal mode yourself on your desktop session:

1. Refresh and select the intended paired phone. Enter a recipient you control.
2. Paste text and a small clipboard image. Confirm that both remain in the draft.
3. Review the sender phone, recipient, attachment count, and default-SIM notice.
   Cancel once and verify that the draft is retained.
4. Send only when you intend to transmit an actual SMS/MMS. Verify the message on
   both phones; desktop acceptance alone does not establish delivery.
5. Check plain-text copying/pasting and image copying/pasting between this app
   and another app on your actual Wayland session.

Carrier service and the Android app's SMS/MMS permissions are still required.
This composer does not send RCS messages. Never automate live phone verification.

## Porting order

1. SMS composition and image paste through the compatibility adapter.
2. Conversation history, incoming messages, contacts, and attachment viewing.
3. Native identity storage, discovery, mutually authenticated TLS, explicit
   pairing and unpairing, and reconnect behavior.
4. Native SMS/MMS transport and plugin capability negotiation.
5. Remaining plugins: clipboard sync, notifications, sharing, battery and
   connectivity, media/volume, input, commands, find-phone, SFTP, and monitoring.
6. Desktop integration, packaging, accessibility, localization, and parity tests
   before switching away from the original daemon.

Tests must cover rejected and malformed input, attachment lifetimes, oversize
drafts, text paste, image paste, cancellation, failure isolation, and duplicate
send prevention. Live device tests are manual and must not run in CI.
