use loramesh::radio::{
    Action, Controller, ControllerConfig, State,
    protocol::{LineCodec, MAX_LINE, RadioProfile, Reply},
};
use loramesh::sim::{
    device::{Device, Effect, Firmware},
    scenario::controller_config,
};

fn initialized() -> Controller {
    let mut c = Controller::new(controller_config(&RadioProfile {
        sf: 7,
        ..RadioProfile::default()
    }))
    .unwrap();
    c.connected(0);
    let mut actions = c.tick(1_000_000);
    // Independent scripted responses, not the virtual device implementation.
    for _ in 0..100 {
        let command = actions.iter().find_map(|a| {
            if let Action::Write(c) = a {
                Some(c.clone())
            } else {
                None
            }
        });
        match command {
            None => break,
            Some(command) => {
                let response = match command.as_str() {
                    "sys get ver" => "RN2903 1.0.5 Jan 1 2020",
                    "mac pause" => "4294967295",
                    "radio get mod" => "lora",
                    "radio get freq" => "915000000",
                    "radio get sf" => "sf7",
                    "radio get bw" => "125",
                    "radio get cr" => "4/5",
                    "radio get prlen" => "8",
                    "radio get crc" => "on",
                    "radio get sync" => "12",
                    _ if command.starts_with("radio set ")
                        || command == "radio rx 0"
                        || command == "mac reset" =>
                    {
                        "ok"
                    }
                    _ => panic!("unexpected command {}", command),
                };
                actions = c.on_line(response, 1_000_000);
            }
        }
    }
    assert_eq!(c.state(), State::Receiving);
    c
}
fn has_write(actions: &[Action], line: &str) -> bool {
    actions.iter().any(|a| a == &Action::Write(line.into()))
}
#[test]
fn receive_prefix_and_malformed_data() {
    assert_eq!(
        Reply::parse("radio_rx 0001090100").unwrap(),
        Reply::Rx(vec![0, 1, 9, 1, 0])
    );
    assert_eq!(
        Reply::parse("radio_rx  00FF \r\n").unwrap(),
        Reply::Rx(vec![0, 255])
    );
    for bad in [
        "radio_rx",
        "radio_rx ",
        "radio_rx0A",
        "radio_rx 1",
        "radio_rx zz",
        "",
        "ok\0",
    ] {
        assert!(Reply::parse(bad).is_err(), "{}", bad);
    }
    assert!(Reply::parse(&format!("radio_rx {}", "aa".repeat(256))).is_err());
}
#[test]
fn fragmented_combined_and_oversized_lines_resynchronize() {
    let mut codec = LineCodec::default();
    let mut lines = vec![];
    for part in [b"radio_".as_ref(), b"rx 00\r", b"\nok\r\n"] {
        for b in part {
            if let Some(line) = codec.push(*b) {
                lines.push(line.unwrap());
            }
        }
    }
    assert_eq!(lines, vec!["radio_rx 00", "ok"]);
    let mut errors = 0;
    for b in vec![b'x'; MAX_LINE * 10] {
        if codec.push(b).is_some() {
            errors += 1;
        }
    }
    assert_eq!(errors, 1);
    assert!(codec.push(b'\n').is_none());
    let mut last = None;
    for b in b"ok\r\n" {
        if let Some(line) = codec.push(*b) {
            last = Some(line.unwrap());
        }
    }
    assert_eq!(last.as_deref(), Some("ok"));
}
#[test]
fn independent_airtime_vectors() {
    let mut p = RadioProfile::default();
    assert_eq!(p.airtime_us(207).unwrap(), 7_544_832);
    p.sf = 9;
    assert_eq!(p.airtime_us(207).unwrap(), 1_045_504);
    p.sf = 7;
    assert_eq!(p.airtime_us(207).unwrap(), 327_936);
    assert_eq!(p.airtime_us(20).unwrap(), 56_576);
    p.bandwidth = 500_000;
    assert_eq!(p.airtime_us(207).unwrap(), 81_984);
    assert!(p.airtime_us(256).is_err());
    p.sf = 0;
    assert!(p.airtime_us(20).is_err());
}
#[test]
fn one_submission_one_tx_with_receive_race() {
    let mut c = initialized();
    c.enqueue(7, vec![1, 2], 1_000_000 + 100).unwrap();
    assert!(has_write(&c.tick(1_000_000 + 100), "radio rxstop"));
    let a = c.on_line("radio_rx 00ff", 1_000_000 + 110);
    assert_eq!(a, vec![Action::Received(vec![0, 255])]);
    assert_eq!(c.state(), State::StoppingReceive);
    let a = c.on_line("ok", 1_000_000 + 120);
    assert!(has_write(&a, "radio tx 0102"));
    c.on_line("ok", 1_000_000 + 130);
    assert_eq!(c.state(), State::Transmitting);
    let a = c.on_line("radio_tx_ok", 1_000_000 + 50000);
    assert!(a.contains(&Action::Transmitted(7)));
    assert!(has_write(&a, "radio rx 0"));
    c.on_line("ok", 1_000_000 + 50010);
    for t in [60000, 100000, 1000000] {
        assert!(
            !c.tick(1_000_000 + t)
                .iter()
                .any(|a| matches!(a, Action::Write(_)))
        );
    }
    assert_eq!(c.queued(), 0);
}
#[test]
fn tx_completion_timeout_fails_once_and_reconnects() {
    let mut c = initialized();
    c.enqueue(8, vec![1], 1_000_000).unwrap();
    c.tick(1_000_000);
    c.on_line("ok", 1_000_000 + 1);
    c.on_line("ok", 1_000_000 + 2);
    let deadline = c.next_deadline().unwrap();
    let a = c.tick(deadline);
    assert_eq!(
        a.iter()
            .filter(|a| matches!(a, Action::Failed(8, _)))
            .count(),
        1
    );
    assert_eq!(c.state(), State::Recovering);
    assert!(c.on_line("radio_tx_ok", deadline + 1).is_empty());
    assert!(c.tick(deadline + 1_000_000).contains(&Action::Reconnect));
    assert_eq!(c.queued(), 0);
}
#[test]
fn missing_ack_busy_and_watchdog_recover() {
    let mut c = initialized();
    c.enqueue(1, vec![4], 1_000_000).unwrap();
    c.tick(1_000_000);
    let a = c.on_line("busy", 1_000_000 + 1);
    assert!(a.iter().any(|a| matches!(a, Action::Failed(1, _))));
    let mut c = initialized();
    c.enqueue(2, vec![4], 1_000_000).unwrap();
    c.tick(1_000_000);
    c.on_line("ok", 1_000_000 + 1);
    c.on_line("ok", 1_000_000 + 2);
    assert!(
        c.on_line("radio_err", 1_000_000 + 3)
            .iter()
            .any(|a| matches!(a, Action::Failed(2, _)))
    );
    let mut c = initialized();
    c.enqueue(3, vec![4], 1_000_000).unwrap();
    c.tick(1_000_000);
    assert!(
        c.tick(1_000_000 + 2_000_000)
            .iter()
            .any(|a| matches!(a, Action::Failed(3, _)))
    );
    let mut c = initialized();
    assert!(has_write(&c.on_line("radio_err", 1_000_000), "radio rx 0"));
}
#[test]
fn asynchronous_events_do_not_consume_initialization_response() {
    let config = ControllerConfig {
        initialization: vec!["sys set pindig GPIO10 1".into()],
        ..Default::default()
    };
    let mut c = Controller::new(config).unwrap();
    c.connected(0);
    c.tick(1_000_000);
    assert!(has_write(
        &c.on_line("RN2903 1.0.5 virtual", 1_000_000),
        "mac reset"
    ));
    assert!(has_write(
        &c.on_line("ok", 1_000_000),
        "sys set pindig GPIO10 1"
    ));
    assert_eq!(
        c.on_line("radio_rx 0102", 1_000_001),
        vec![Action::Received(vec![1, 2])]
    );
    assert!(has_write(&c.on_line("ok", 1_000_002), "radio get mod"));
}
#[test]
fn queue_caps_age_and_shutdown() {
    let config = ControllerConfig {
        queue_frames: 1,
        max_packet_age_us: 100,
        ..Default::default()
    };
    let mut c = Controller::new(config).unwrap();
    c.enqueue(1, vec![1], 0).unwrap();
    assert!(c.enqueue(2, vec![2], 0).is_err());
    assert!(
        c.tick(100)
            .iter()
            .any(|a| matches!(a, Action::Failed(1, _)))
    );
    c.enqueue(3, vec![3], 101).unwrap();
    assert!(
        c.shutdown()
            .iter()
            .any(|a| matches!(a, Action::Failed(3, _)))
    );
    assert!(c.enqueue(4, vec![4], 102).is_err());
    assert!(c.tick(u64::MAX).is_empty());
    let config = ControllerConfig {
        queue_airtime_us: 1,
        ..Default::default()
    };
    let mut c = Controller::new(config).unwrap();
    assert!(c.enqueue(1, vec![1], 0).is_err());
}
#[test]
fn invalid_configuration_is_rejected_before_io() {
    for line in [
        "radio set sf sf99",
        "radio set bw 0",
        "radio set crc invalid",
        "radio tx 00",
        "sys eraseFW",
        "radio rx 0\r\nsys reset",
    ] {
        let config = ControllerConfig {
            initialization: vec![line.into()],
            ..Default::default()
        };
        assert!(Controller::new(config).is_err(), "{}", line);
    }
}
#[test]
fn independent_virtual_device_conformance_fixture() {
    let fixture: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/rn2903.json")).unwrap();
    let mut device = Device::new(Firmware::RN2903, RadioProfile::default());
    for step in fixture {
        let command = step["command"].as_str().unwrap();
        let expected = step["reply"].as_str().unwrap();
        let out = device.command(command);
        let reply = out.iter().find_map(|e| {
            if let Effect::Line(l) = e {
                Some(l.as_str())
            } else {
                None
            }
        });
        assert_eq!(reply, Some(expected), "{}", command);
    }
}
#[test]
fn rn2483_profile_and_invalid_commands() {
    let profile = RadioProfile {
        frequency: 868_000_000,
        ..Default::default()
    };
    let mut d = Device::new(Firmware::RN2483, profile);
    for (command, expected) in [
        ("mac reset", "invalid_param"),
        ("mac reset 868", "ok"),
        ("radio set pwr 20", "invalid_param"),
        ("radio set pwr 14", "ok"),
        ("radio set freq 915000000", "invalid_param"),
        ("unsupported command", "invalid_param"),
    ] {
        assert!(matches!(&d.command(command)[0],Effect::Line(s) if s==expected));
    }
}
#[test]
fn malformed_rx_and_rx_before_ack_rearm_receiver() {
    let mut c = initialized();
    assert!(has_write(
        &c.on_line("radio_rx zz", 1_000_001),
        "radio rx 0"
    ));
    assert!(
        c.on_line("radio_rx 01", 1_000_002)
            .contains(&Action::Received(vec![1]))
    );
    assert!(has_write(&c.on_line("ok", 1_000_003), "radio rx 0"));
    c.on_line("radio_err", 1_000_004);
    assert!(has_write(&c.on_line("ok", 1_000_005), "radio rx 0"));
    c.on_line("ok", 1_000_006);
    assert_eq!(c.state(), State::Receiving);
}
#[test]
fn virtual_reset_restores_profile_and_rejects_oversized_tx() {
    let mut d = Device::new(Firmware::RN2903, RadioProfile::default());
    d.command("radio set sf sf7");
    d.command("sys reset");
    assert_eq!(d.profile.sf, 12);
    d.command("mac pause");
    assert!(
        matches!(&d.command(&format!("radio tx {}","00".repeat(256)))[0],Effect::Line(s) if s=="invalid_param")
    );
}

#[test]
fn startup_drains_stale_replies_without_rebooting() {
    let mut c = Controller::new(ControllerConfig::default()).unwrap();
    assert!(has_write(&c.connected(0), "INVALIDCOMMAND"));
    for line in [
        "invalid_param",
        "ok",
        "radio_err",
        "radio_rx 1234",
        "RN2903 stale",
    ] {
        assert!(c.on_line(line, 999_999).is_empty());
    }
    assert!(c.tick(999_999).is_empty());
    assert!(has_write(&c.tick(1_000_000), "sys get ver"));
    assert!(has_write(
        &c.on_line("RN2903 1.0.5", 1_000_001),
        "mac reset"
    ));
    assert!(has_write(&c.on_line("ok", 1_000_002), "mac pause"));
}

#[test]
fn rn2483_startup_selects_configured_band_without_module_reset() {
    for frequency in [433_500_000, 868_000_000] {
        let profile = RadioProfile {
            frequency,
            ..Default::default()
        };
        let mut c = Controller::new(controller_config(&profile)).unwrap();
        let mut d = Device::new(Firmware::RN2483, profile);
        c.connected(0);
        let mut actions = c.tick(1_000_000);
        let mut commands = Vec::new();
        for _ in 0..100 {
            let Some(command) = actions.iter().find_map(|a| match a {
                Action::Write(line) => Some(line.clone()),
                _ => None,
            }) else {
                break;
            };
            commands.push(command.clone());
            actions = d
                .command(&command)
                .into_iter()
                .flat_map(|e| match e {
                    Effect::Line(line) => c.on_line(&line, 1_000_000),
                    _ => vec![],
                })
                .collect();
        }
        assert_eq!(c.state(), State::Receiving, "{commands:?}");
        assert!(!commands.iter().any(|c| c == "sys reset"));
        assert_eq!(d.generation, 0);
        assert_eq!(c.profile().frequency, frequency);
    }
}

#[test]
fn one_lingering_rx_error_before_tx_ack_is_tolerated_but_bounded() {
    for outcome in ["success", "second_error", "no_ack", "tx_error"] {
        let mut c = initialized();
        c.enqueue(1, vec![42], 1_000_000).unwrap();
        c.tick(1_000_000);
        c.on_line("ok", 1_000_001);
        let deadline = c.next_deadline();
        let a = c.on_line("radio_err", 1_000_002);
        assert!(
            !a.iter()
                .any(|a| matches!(a, Action::Failed(..) | Action::Close))
        );
        assert_eq!(c.state(), State::AwaitingTxAck);
        assert_eq!(c.next_deadline(), deadline);
        let result = match outcome {
            "second_error" => c.on_line("radio_err", 1_000_003),
            "no_ack" => c.tick(deadline.unwrap()),
            _ => {
                c.on_line("ok", 1_000_003);
                c.on_line(
                    if outcome == "success" {
                        "radio_tx_ok"
                    } else {
                        "radio_err"
                    },
                    1_050_000,
                )
            }
        };
        if outcome == "success" {
            assert!(result.contains(&Action::Transmitted(1)));
        } else {
            assert_eq!(
                result
                    .iter()
                    .filter(|a| matches!(a, Action::Failed(1, _)))
                    .count(),
                1
            );
            assert!(!result.iter().any(|a| matches!(a, Action::Transmitted(_))));
        }
    }
}
