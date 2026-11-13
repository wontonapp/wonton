use qrcode::{EcLevel, QrCode};
use wonton_tunnel::pair::{PairingCode, PairingSecret, SECRET_LEN};
use wonton_tunnel::relays::RelayTable;

use super::{Issued, format_pairing};

fn issued() -> Issued {
    Issued {
        code: PairingCode {
            server_id: iroh::SecretKey::from_bytes(&[1; 32]).public(),
            relays: RelayTable::parse(
                "https://aps1-1.relay.n0.iroh.link,https://euc1-1.relay.n0.iroh.link,https://usw1-1.relay.n0.iroh.link",
            )
            .unwrap()
            .relays()
            .to_vec(),
            direct_addrs: vec!["192.0.2.10:4433".parse().unwrap()],
            base_domain: "wonton.localhost".to_owned(),
            core_url: "http://wonton.localhost:8480/".parse().unwrap(),
            secret: PairingSecret::from_bytes([2; SECRET_LEN]),
        }
        .to_uri(),
        expires_in: 600,
    }
}

fn has_qr_blocks(text: &str) -> bool {
    text.contains(['█', '▀', '▄'])
}

#[test]
fn qr_fits_at_the_exact_minimum_width_including_quiet_zone() {
    let issued = issued();
    let code = QrCode::with_error_correction_level(issued.code.as_bytes(), EcLevel::L).unwrap();
    let required = code.width() + 8;
    let medium = QrCode::with_error_correction_level(issued.code.as_bytes(), EcLevel::M).unwrap();
    assert!(medium.width() + 8 > required);

    for columns in [required, required + 1] {
        let output = format_pairing(&issued, "", Some(columns.try_into().unwrap()));
        let rows: Vec<_> = output.lines().filter(|line| has_qr_blocks(line)).collect();
        assert_eq!(rows.len(), required.div_ceil(2));
        for row in &rows {
            assert_eq!(row.chars().count(), required);
            assert!(row.chars().count() <= columns);
        }
        // 四模块白色静区：上下各两行，左右各四列；最后半行只画上半格。
        assert!(rows[..2].iter().all(|row| row.chars().all(|c| c == '█')));
        assert!(rows[rows.len() - 2].chars().all(|c| c == '█'));
        assert!(rows.last().unwrap().chars().all(|c| c == '▀'));
        for row in &rows[..rows.len() - 1] {
            assert!(row.starts_with("████") && row.ends_with("████"));
        }
        assert!(output.lines().any(|line| line == issued.code));
    }
}

#[test]
fn too_narrow_terminal_keeps_the_text_code_without_qr_blocks() {
    let issued = issued();
    let required = QrCode::with_error_correction_level(issued.code.as_bytes(), EcLevel::L)
        .unwrap()
        .width()
        + 8;
    for columns in [0, required - 1] {
        let output = format_pairing(&issued, "", Some(columns.try_into().unwrap()));
        assert!(!has_qr_blocks(&output));
        assert!(output.lines().any(|line| line == issued.code));
    }
}

#[test]
fn non_terminal_output_keeps_both_qr_and_text_code() {
    let issued = issued();
    let output = format_pairing(&issued, "", None);
    assert!(has_qr_blocks(&output));
    assert!(output.lines().any(|line| line == issued.code));
}
