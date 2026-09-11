use super::*;

fn state() -> ConsoleState {
    let rows: &[(u16, u16, &[u8])] = &[
        (0x8023, 0, b""),
        (0x8023, 1, b""),
        (0x03e8, 10, &[1]),
        (0x03e8, 11, &[0]),
        (0x0420, 0, &[1, 0, 0, 0]),
        (0x0403, 0, &[1, 0, 0, 0]),
        (0x0403, 25, &[1, 0, 0, 0]),
        (0x841a, 0x0a00, &[0, 0x40, 0x4d, 0xe6]),
        (0x83f8, 0x0a00, &[0, 0x40, 0x4d, 0xe6]),
        (0x83f8, 0x0a19, &[0, 0, 0, 0]),
        (0x842b, 0x0a00, &[0, 0x80, 0, 0]),
        (0x03fa, 10, &[0]),
        (0x03fb, 10, &[0]),
        (0x93ac, 0, &[0x13, 0xad, 1, 0]),
        (0x93ac, 1, &[0x13, 0xad, 1, 1]),
    ];
    ConsoleState::from_records(
        rows.iter()
            .map(|&(property_id, property_index, value)| VendorStateRecord {
                property_id,
                property_index,
                value: value.to_vec(),
            })
            .collect(),
    )
    .unwrap()
}

#[test]
fn q24_matches_both_captured_faders_and_client_converter() {
    assert_eq!(encode_level("-12").unwrap(), 0x0040_4de6);
    assert_eq!(encode_level("-60").unwrap(), 0x0000_4189);
    assert_eq!(encode_level("0").unwrap(), 0x0100_0000);
    assert_eq!(encode_level("-inf").unwrap(), 0);
    assert_eq!(encode_level("-6").unwrap(), 0x0080_4dce);
    assert_eq!(encode_pan("-1").unwrap(), 0);
    assert_eq!(encode_pan("0").unwrap(), 0x0080_0000);
    assert_eq!(encode_pan("1").unwrap(), 0x0100_0000);
    for bad in ["NaN", "inf", "-91", "12.1", ""] {
        assert!(encode_level(bad).is_err());
    }
    for bad in ["NaN", "inf", "-1.1", "1.1"] {
        assert!(encode_pan(bad).is_err());
    }
}

#[test]
fn selects_input_and_bus_bytes_without_confusing_mix_slots() {
    let console = state();
    let changes = parse_changes("level:main:10:00404de6:-60;level:aux-25:10:00000000:-6").unwrap();
    let writes = console.prepare(&changes).unwrap();
    assert_eq!((writes[0].property, writes[0].index), (0x841a, 0x0a00));
    assert_eq!((writes[1].property, writes[1].index), (0x83f8, 0x0a19));
    assert!(console
        .prepare(&parse_changes("level:aux-26:10:00000000:0").unwrap())
        .is_err());
    assert!(console
        .prepare(&parse_changes("level:main:63:00000000:0").unwrap())
        .is_err());
}

#[test]
fn rejects_entire_stale_or_invalid_route_batch_before_writing() {
    let console = state();
    let valid = "route:line:0:13ad0100:13b00000";
    let writes = console.prepare(&parse_changes(valid).unwrap()).unwrap();
    assert_eq!(writes[0].value, vec![0x13, 0xb0, 0, 0]);
    for invalid in [
        "route:line:1:00000000:13b00001",    // stale prior value
        "route:line:1:13ad0101:13b000ff",    // nonexistent source
        "route:line:12:13ad0101:13b00001",   // nonexistent destination
        "route:monitor:0:00000000:13b00001", // monitor is not advertised
    ] {
        assert!(console
            .prepare(&parse_changes(&format!("{valid};{invalid}")).unwrap())
            .is_err());
    }
    assert!(console
        .prepare(&parse_changes(&format!("{valid};{valid}")).unwrap())
        .is_err());
}

#[test]
fn no_op_values_do_not_send_setters() {
    let console = state();
    assert!(console
        .prepare(&parse_changes("level:main:10:00404de6:-12;mute:input:10:00:0").unwrap())
        .unwrap()
        .is_empty());
}

#[test]
fn bounds_batches_and_untrusted_values() {
    for bad in [
        "",
        "route:line:0:00",
        "route:line:0:xx:00000000",
        "mute:input:65536:00:1",
        "level:main:0:💥:0",
    ] {
        assert!(parse_changes(bad).is_err(), "{bad}");
    }
    assert!(parse_changes(&vec!["mute:input:0:00:1"; 33].join(";")).is_err());
    assert!(state()
        .prepare(&parse_changes("mute:input:10:00:2").unwrap())
        .is_err());
}

#[test]
fn excludes_unmapped_properties_and_rejects_duplicate_state() {
    let record = VendorStateRecord {
        property_id: 0x03e8,
        property_index: 0,
        value: vec![0],
    };
    assert!(ConsoleState::from_records(vec![record.clone(), record]).is_err());
    assert!(!relevant(0x1395)); // unrelated monitor setting remains unmapped
    assert!(!relevant(0x001f)); // preset commands
}

fn monitor_state() -> ConsoleState {
    let mut console = state();
    for (p, i, v) in [
        (0x13b6, 0, vec![0]),
        (0x1393, 0, vec![30]),
        (0x1394, 0, vec![0, 3]),
        (0x93b9, 0, vec![0; 4]),
        (0x93b9, 1, vec![0; 4]),
        (0x93ad, 0, vec![0; 4]),
    ] {
        console.records.insert((p, i), v);
    }
    for i in 0..12 {
        console.records.insert((0x1388, i), vec![0]);
    }
    console
}

#[test]
fn monitor_controls_match_selection_bitmask_and_captured_membership_bytes() {
    let console = monitor_state();
    for mask in 1..=7 {
        let writes = console
            .prepare(&parse_changes(&format!("monitor-select:monitor:0:00:{mask}")).unwrap())
            .unwrap();
        assert_eq!((writes[0].property, writes[0].index), (0x13b6, 0));
        assert_eq!(writes[0].value, [mask]);
    }
    let writes = console
        .prepare(
            &parse_changes("monitor-members:monitor:0:0003:2314;monitor-level:monitor:0:1e:-inf")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(writes[0].value, [9, 10]); // Captured members 2, 4, 9, 12.
    assert_eq!((writes[1].property, &writes[1].value), (0x1393, &vec![100]));
    assert!(console.prepare(&parse_changes("monitor-select:monitor:0:00:0;monitor-level:monitor:0:1e:-30;monitor-members:monitor:0:0003:3").unwrap()).unwrap().is_empty());
}

#[test]
fn rejects_stale_monitor_state_unmapped_masks_and_invalid_levels() {
    for invalid in [
        "monitor-select:monitor:0:00:-1",
        "monitor-select:monitor:0:00:255",
        "monitor-select:monitor:0:00:256",
        "monitor-select:monitor:0:00:8",
        "monitor-select:monitor:1:00:1",
        "monitor-select:main:0:00:1",
        "monitor-select:monitor:0:01:2",
        "monitor-level:monitor:0:1e:1",
        "monitor-level:monitor:0:1e:-100",
        "monitor-level:monitor:0:1e:NaN",
        "monitor-level:monitor:0:1e:-10.5",
        "monitor-members:monitor:0:0003:4096",
        "monitor-members:monitor:0:0001:3",
    ] {
        assert!(
            monitor_state()
                .prepare(&parse_changes(invalid).unwrap())
                .is_err(),
            "{invalid}"
        );
    }
    let mut console = monitor_state();
    console.records.insert((0x13b6, 0), vec![3]); // A+B can be changed or turned off.
    assert!(console
        .prepare(&parse_changes("monitor-select:monitor:0:03:0").unwrap())
        .is_ok());
    console.records.insert((0x1394, 0), vec![0x10, 3]);
    assert!(console
        .prepare(&parse_changes("monitor-members:monitor:0:1003:3").unwrap())
        .is_err());
}

#[test]
fn speaker_combinations_switch_with_one_mask_and_preserve_conflict_checks() {
    let mut console = monitor_state();
    for (old, new) in [(1, 3), (3, 5), (5, 6), (6, 7), (7, 0)] {
        console.records.insert((0x13b6, 0), vec![old]);
        let changes = parse_changes(&format!("monitor-select:monitor:0:{old:02x}:{new}")).unwrap();
        let writes = console.prepare(&changes).unwrap();
        assert_eq!(
            writes,
            vec![WriteRecord {
                property: 0x13b6,
                index: 0,
                value: vec![new]
            }]
        );
        console.records.insert((0x13b6, 0), vec![new]);
        assert!(console
            .prepare(&changes)
            .unwrap_err()
            .starts_with("conflict:"));
    }
}

#[test]
fn front_panel_buttons_require_advertised_booleans_and_reject_batch_conflicts() {
    let mut console = monitor_state();
    for (operation, property) in [
        ("monitor-mute", 0x139b),
        ("monitor-mono", 0x139a),
        ("monitor-talk", 0x13a3),
    ] {
        let enable = parse_changes(&format!("{operation}:monitor:0:00:1")).unwrap();
        assert!(is_monitor_changes(&enable));
        assert!(console.prepare(&enable).is_err()); // Not advertised yet.
        for malformed in [vec![2], vec![0, 0]] {
            console.records.insert((property, 0), malformed);
            assert!(console.prepare(&enable).is_err());
        }
        console.records.insert((property, 0), vec![0]);
        assert_eq!(
            console.prepare(&enable).unwrap(),
            vec![WriteRecord {
                property,
                index: 0,
                value: vec![1]
            }]
        );
        for invalid in [
            format!("{operation}:monitor:0:00:2"),
            format!("{operation}:monitor:0:00:true"),
            format!("{operation}:monitor:1:00:1"),
            format!("{operation}:main:0:00:1"),
        ] {
            assert!(console.prepare(&parse_changes(&invalid).unwrap()).is_err());
        }
        console.records.insert((property, 0), vec![1]);
        assert!(console
            .prepare(&enable)
            .unwrap_err()
            .starts_with("conflict:"));
        let disable = parse_changes(&format!("{operation}:monitor:0:01:0")).unwrap();
        assert_eq!(console.prepare(&disable).unwrap()[0].value, [0]);
    }
    let stale_batch =
        parse_changes("monitor-mute:monitor:0:01:0;monitor-talk:monitor:0:00:1").unwrap();
    assert!(console
        .prepare(&stale_batch)
        .unwrap_err()
        .starts_with("conflict:"));
    assert_eq!(console.get(0x1393, 0).unwrap(), [30]);
    assert_eq!(console.get(0x13b6, 0).unwrap(), [0]);
}

#[test]
fn monitor_routing_requires_stereo_inventory_and_rejects_feedback() {
    let mut console = monitor_state();
    let changes =
        parse_changes("route:monitor:1:00000000:13b00001;route:line:1:13ad0101:13b90201").unwrap();
    let writes = console.prepare(&changes).unwrap();
    assert_eq!((writes[0].property, writes[0].index), (0x93b9, 1));
    assert_eq!(writes[1].value, [0x13, 0xb9, 2, 1]);
    for invalid in [
        "route:monitor:0:00000000:13b90000",
        "route:mixer:0:00000000:13b90000",
        "route:line:1:13ad0101:13b90300",
        "route:line:1:13ad0101:13b90002",
        "route:monitor:2:00000000:13b00000",
    ] {
        assert!(
            console.prepare(&parse_changes(invalid).unwrap()).is_err(),
            "{invalid}"
        );
    }
    console.records.remove(&(0x93b9, 1));
    assert!(!console.monitor_available());
    assert!(console.prepare(&changes).is_err());
}

#[test]
fn uses_client_proven_mute_solo_ids_and_rejects_failed_acknowledgements() {
    let writes = state()
        .prepare(&parse_changes("mute:input:10:00:1;solo:input:10:00:1").unwrap())
        .unwrap();
    assert_eq!(writes[0].property, 0x03fb); // kiMixMute, CueMix 0x1405787f0
    assert_eq!(writes[1].property, 0x03fa); // kiMixSolo, CueMix 0x140578230
    let mut ack = vec![0; 28];
    ack[3] = 16;
    assert!(validate_ack(&ack).is_ok());
    ack[2] = 8;
    assert!(validate_ack(&ack).unwrap_err().contains("status 1"));
    ack[2] = 0;
    ack[3] = 17;
    assert!(validate_ack(&ack).is_err());
    assert!(validate_ack(&ack[..20]).is_err());
}

#[test]
fn source_inventory_preserves_banked_names_and_sparse_destinations() {
    let mut console = state();
    for (property, index, value) in [
        (0x8023, 0x0f07, vec![]),
        (0x8022, 0x0107, vec![]),
        (0x8024, 127, vec![]),
        (0x1b5b, 15, vec![0, 1, 0x77, 0, 8, 0]),
        (0x93af, 0x0f07, vec![0, 0, 0, 0]),
        (0x93ae, 0x0107, vec![0, 0, 0, 0]),
        (0x93b1, 0x0101, vec![0, 0, 0, 0]),
    ] {
        console.records.insert((property, index), value);
    }
    let changes = parse_changes("route:network:3847:00000000:13b0007f;route:optical:263:00000000:13af0f07;route:phones:257:00000000:13ae0107").unwrap();
    let writes = console.prepare(&changes).unwrap();
    assert_eq!(
        writes.iter().map(|r| r.index).collect::<Vec<_>>(),
        vec![0x0f07, 0x0107, 0x0101]
    );
    console.records.get_mut(&(0x1b5b, 15)).unwrap()[4] = 0;
    assert!(console.prepare(&changes).is_err());
}

#[test]
fn master_and_pre_controls_address_each_selected_bus_channel() {
    let mut console = state();
    for (p, i, v) in [
        (0x0420, 1, vec![1, 0, 0, 0]),
        (0x0403, 1, vec![1, 0, 0, 0]),
        (0x0411, 1, vec![0]),
    ] {
        console.records.insert((p, i), v);
    }
    let writes = console
        .prepare(
            &parse_changes(
                "master:main:1:01000000:-12;master:aux-1:1:01000000:-6;pre:aux-1:1:00:1",
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        writes
            .iter()
            .map(|r| (r.property, r.index))
            .collect::<Vec<_>>(),
        vec![(0x0420, 1), (0x0403, 1), (0x0411, 1)]
    );
    for invalid in [
        "pre:main:0:00:1",
        "master:aux-0:1:01000000:-6",
        "master:main:2:01000000:-6",
    ] {
        assert!(console.prepare(&parse_changes(invalid).unwrap()).is_err());
    }
}
