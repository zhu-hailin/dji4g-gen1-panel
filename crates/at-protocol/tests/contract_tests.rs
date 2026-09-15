//! 阶段 0 共享契约测试：钉住扩展文档（研究文档附录 A / §2.2）定义的解析契约，供阶段 A/B/C
//! 子代理共同遵守。改动这些断言需要先在主执行者处协调。

use dji4g_at_protocol::{
    CnumParseError, at_csv, parse_cnum_lines, parse_iccid_line, parse_serving_cell_line,
};
use dji4g_domain::{NumberLookup, ServingCell};

const LTE18: &str = "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,1A2B3C4,123,1650,3,5,5,0ABC,-95,-10,-65,15,20";

#[test]
fn qeng_18_field_layout_parses_hex_cellid_without_pci_shift() {
    let ServingCell {
        state,
        duplex,
        rat,
        mcc,
        mnc,
        cell_id,
        pci,
        earfcn,
        band,
        ul_mhz,
        dl_mhz,
        tac,
        rsrp_dbm,
        rsrq_db,
        rssi_dbm,
        sinr_raw,
        srxlev_raw,
    } = parse_serving_cell_line(LTE18).expect("reference layout must parse");
    assert_eq!(state.as_deref(), Some("NOCONN"));
    assert_eq!(duplex.as_deref(), Some("FDD"));
    assert_eq!(rat.as_deref(), Some("LTE"));
    assert_eq!(mcc.as_deref(), Some("460"));
    assert_eq!(mnc.as_deref(), Some("01"));
    assert_eq!(cell_id, Some(0x1A2B3C4));
    assert_eq!(pci, Some(123));
    assert_eq!(earfcn, Some(1650));
    assert_eq!(band, Some(3));
    // Bandwidth index 5 → 20 MHz (0..=5 → 1.4/3/5/10/15/20).
    assert_eq!(ul_mhz, Some(20.0));
    assert_eq!(dl_mhz, Some(20.0));
    assert_eq!(tac, Some(0xABC));
    assert_eq!(rsrp_dbm, Some(-95));
    assert_eq!(rsrq_db, Some(-10));
    assert_eq!(rssi_dbm, Some(-65));
    assert_eq!(sinr_raw, Some(15));
    assert_eq!(srxlev_raw, Some(20));
}

#[test]
fn qeng_search_is_a_valid_state_not_malformed() {
    for state in ["SEARCH", "LIMSRV", "NOCELL"] {
        let line = format!("+QENG: \"servingcell\",\"{state}\"");
        let cell = parse_serving_cell_line(&line).expect("state-only report is valid");
        assert_eq!(cell.rat, None);
        assert_eq!(cell.state.as_deref(), Some(state));
    }
}

#[test]
fn qeng_missing_metric_degrades_to_none_not_a_dropped_cell() {
    let line = LTE18.replace(",-95,-10,-65,15,20", ",-,-,-,-,-");
    let cell = parse_serving_cell_line(&line).expect("missing metrics must not drop the cell");
    assert_eq!(cell.rsrp_dbm, None);
    assert_eq!(cell.rsrq_db, None);
    assert_eq!(cell.rssi_dbm, None);
    assert_eq!(cell.sinr_raw, None);
    assert_eq!(cell.pci, Some(123));
}

#[test]
fn qeng_old_16_field_layout_is_rejected_not_misread() {
    // 旧布局（pci 在第 6 字段）与参考 18 字段布局冲突；必须整体拒绝，不能把 cellid 当 PCI。
    let old = "+QENG: \"servingcell\",1,\"LTE\",1,460,01,351,1300,3,20,20,9365,-95,-8,-63,13";
    assert!(parse_serving_cell_line(old).is_none());
}

#[test]
fn cnum_empty_ok_is_not_a_failure() {
    assert_eq!(parse_cnum_lines(&[]).unwrap(), NumberLookup::Empty);
}

#[test]
fn cnum_parses_quoted_labels_with_commas_and_masks_numbers() {
    let lookup = parse_cnum_lines(&["+CNUM: \"line,1\",\"+12025550123\",145"]).unwrap();
    let debug = format!("{lookup:?}");
    let NumberLookup::Reported(numbers) = lookup else {
        panic!("expected a reported number");
    };
    assert_eq!(numbers[0].expose_after_user_action(), "+12025550123");
    assert_eq!(numbers[0].masked(), "****0123");
    assert_eq!(numbers[0].toa, 145);
    assert!(!debug.contains("12025550123"));
}

#[test]
fn cnum_keeps_multiple_records_and_deduplicates() {
    let lookup = parse_cnum_lines(&[
        "+CNUM: ,\"+12025550123\",145",
        "+CNUM: ,\"+12025550124\",145",
        "+CNUM: ,\"+12025550123\",145",
    ])
    .unwrap();
    let NumberLookup::Reported(numbers) = lookup else {
        panic!("expected numbers");
    };
    assert_eq!(numbers.len(), 2);
}

#[test]
fn cnum_rejects_malformed_shapes_without_silent_guesses() {
    assert!(parse_cnum_lines(&["+CNUM: \"+1\"", "extra"]).is_err());
    assert!(matches!(
        parse_cnum_lines(&["+CNUM: \"\",\"+12025550123\",129,4,5,6,7,8"]),
        Err(CnumParseError::Shape)
    ));
    assert!(matches!(
        parse_cnum_lines(&["+CNUM: \"\",\"a b\",145"]),
        Err(CnumParseError::Number)
    ));
}

#[test]
fn at_csv_handles_commas_empties_and_escaped_quotes() {
    assert_eq!(
        at_csv("\"主卡,号码\",,145").unwrap(),
        ["主卡,号码", "", "145"]
    );
    assert_eq!(at_csv("\"say \"\"hi\"\"\",7").unwrap(), ["say \"hi\"", "7"]);
    assert!(at_csv("\"oops").is_err());
}

#[test]
fn iccid_is_digits_only_and_rejects_other_shapes() {
    assert_eq!(
        parse_iccid_line("+QCCID: \"89860123456789012345\"").as_deref(),
        Some("89860123456789012345")
    );
    assert!(parse_iccid_line("+QCCID: 8986-0123").is_none());
    assert!(parse_iccid_line("+QCCID:").is_none());
}

#[test]
fn command_classification_labels_identity_queries() {
    use dji4g_at_protocol::{AtCommand, Effect, Sensitivity};
    assert_eq!(
        AtCommand::SubscriberNumber.effect(),
        Effect::PureRead,
        "CNUM is read-only, yet still subscriber identity"
    );
    assert_eq!(
        AtCommand::SubscriberNumber.sensitivity(),
        Sensitivity::SubscriberIdentity
    );
    assert_eq!(
        AtCommand::Iccid.sensitivity(),
        Sensitivity::SubscriberIdentity
    );
    assert_eq!(
        AtCommand::RestartModule.effect(),
        Effect::ConnectivityChange
    );
}
