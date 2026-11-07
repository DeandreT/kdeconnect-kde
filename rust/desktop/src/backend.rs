// SPDX-FileCopyrightText: 2026 Deandre Thomas
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, ensure};
use kdeconnect_sms::Draft;
use std::time::Duration;
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedValue, StructureBuilder, Value},
};

#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
}

pub struct Backend {
    connection: Connection,
    destination: String,
}

fn address_value(recipient: &str) -> Result<OwnedValue> {
    Ok(OwnedValue::try_from(Value::from(
        StructureBuilder::new().add_field(recipient).build()?,
    ))?)
}

impl Backend {
    pub fn connect() -> Result<Self> {
        let connection = zbus::blocking::connection::Builder::session()?
            .method_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            connection,
            destination: "org.kde.kdeconnect".into(),
        })
    }

    fn proxy<'a>(&'a self, path: &'a str, interface: &'a str) -> Result<Proxy<'a>> {
        Ok(Proxy::new(
            &self.connection,
            self.destination.as_str(),
            path,
            interface,
        )?)
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        let daemon = self.proxy("/modules/kdeconnect", "org.kde.kdeconnect.daemon")?;
        let ids: Vec<String> = daemon.call("devices", &(true, true))?;
        let mut devices = Vec::new();
        for id in ids {
            let path = format!("/modules/kdeconnect/devices/{id}");
            let device = self.proxy(&path, "org.kde.kdeconnect.device")?;
            let name = device
                .get_property::<String>("name")
                .unwrap_or_else(|_| id.clone());
            devices.push(Device { id, name });
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(devices)
    }

    pub fn send(&self, device: &Device, draft: &Draft) -> Result<()> {
        draft.validate()?;
        let path = format!("/modules/kdeconnect/devices/{}", device.id);
        let status = self.proxy(&path, "org.kde.kdeconnect.device")?;
        ensure!(
            status.get_property::<bool>("isReachable")?,
            "phone is disconnected"
        );
        ensure!(
            status.get_property::<bool>("isPaired")?,
            "phone is not paired"
        );
        let sms_path = format!("{path}/sms");
        let sms = self.proxy(&sms_path, "org.kde.kdeconnect.device.sms")?;
        let addresses = vec![address_value(draft.recipient.trim())?];
        let attachments: Vec<OwnedValue> = draft
            .attachments
            .iter()
            .map(|image| {
                image
                    .path()
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("attachment path is not UTF-8"))
                    .and_then(|path| Ok(OwnedValue::try_from(Value::from(path))?))
            })
            .collect::<Result<_>>()?;
        let _: () = sms.call("sendSms", &(addresses, &draft.text, attachments, -1_i64))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdeconnect_sms::ImageAttachment;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn address_matches_qt_structure() -> Result<()> {
        assert_eq!(
            address_value("+15550100001")?.value_signature().to_string(),
            "(s)"
        );
        Ok(())
    }

    struct MockDaemon;
    #[zbus::interface(name = "org.kde.kdeconnect.daemon")]
    impl MockDaemon {
        #[zbus(name = "devices")]
        fn devices(&self, reachable: bool, paired: bool) -> Vec<String> {
            assert!(reachable && paired);
            vec!["test_phone".into()]
        }
    }

    struct MockDevice;
    #[zbus::interface(name = "org.kde.kdeconnect.device")]
    impl MockDevice {
        #[zbus(property, name = "name")]
        fn name(&self) -> &str {
            "Test phone"
        }
        #[zbus(property, name = "isReachable")]
        fn is_reachable(&self) -> bool {
            true
        }
        #[zbus(property, name = "isPaired")]
        fn is_paired(&self) -> bool {
            true
        }
    }

    struct MockSms(Arc<AtomicUsize>);
    #[zbus::interface(name = "org.kde.kdeconnect.device.sms")]
    impl MockSms {
        #[zbus(name = "sendSms")]
        fn send_sms(
            &self,
            addresses: Vec<OwnedValue>,
            text: String,
            attachments: Vec<OwnedValue>,
            sub_id: i64,
        ) -> zbus::fdo::Result<()> {
            assert_eq!(addresses.len(), 1);
            assert_eq!(addresses[0].value_signature().to_string(), "(s)");
            assert_eq!(sub_id, -1);
            if text == "fail" {
                return Err(zbus::fdo::Error::Failed("simulated failure".into()));
            }
            for path in attachments {
                let path =
                    String::try_from(path).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
                let data =
                    std::fs::read(path).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
                assert_eq!(&data[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
            }
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    #[ignore = "requires a private dbus-run-session and KDECONNECT_RS_TEST_BUS=1"]
    fn mock_service_send_failure_and_corrected_retry() -> Result<()> {
        // This test must be invoked under dbus-run-session, never the user's bus.
        ensure!(
            std::env::var_os("KDECONNECT_RS_TEST_BUS").is_some(),
            "run this test on a private bus"
        );
        let count = Arc::new(AtomicUsize::new(0));
        let name = format!("org.example.KdeConnectRustTest.p{}", std::process::id());
        let _service = zbus::blocking::connection::Builder::session()?
            .name(name.as_str())?
            .serve_at("/modules/kdeconnect", MockDaemon)?
            .serve_at("/modules/kdeconnect/devices/test_phone", MockDevice)?
            .serve_at(
                "/modules/kdeconnect/devices/test_phone/sms",
                MockSms(Arc::clone(&count)),
            )?
            .build()?;
        let backend = Backend {
            connection: Connection::session()?,
            destination: name,
        };
        let devices = backend.devices()?;
        assert_eq!(devices[0].name, "Test phone");
        let mut draft = Draft {
            recipient: "+15550100001".into(),
            text: "valid".into(),
            ..Default::default()
        };
        draft.attach(ImageAttachment::from_rgba(1, 1, &[0; 4])?)?;
        backend.send(&devices[0], &draft)?;
        draft.text = "fail".into();
        assert!(backend.send(&devices[0], &draft).is_err());
        assert!(draft.attachments[0].path().exists());
        draft.text = "corrected".into();
        backend.send(&devices[0], &draft)?;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
