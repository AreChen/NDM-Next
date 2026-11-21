use ndm_core::{PeerStats, SharingStatus};

use crate::view::format_bytes;

pub(crate) fn status(status: SharingStatus, chinese: bool) -> &'static str {
    match (status, chinese) {
        (SharingStatus::Checking, true) => "正在校验文件",
        (SharingStatus::Checking, false) => "Checking files",
        (SharingStatus::Sharing, true) => "做种中",
        (SharingStatus::Sharing, false) => "Seeding",
        (SharingStatus::Stopping, true) => "正在停止做种",
        (SharingStatus::Stopping, false) => "Stopping seeding",
    }
}

pub(crate) fn ratio(peers: PeerStats, downloaded: u64) -> String {
    if downloaded == 0 {
        "—".into()
    } else {
        format!("{:.2}", peers.uploaded as f64 / downloaded as f64)
    }
}

pub(crate) fn details(
    peers: PeerStats,
    downloaded: u64,
    chinese: bool,
) -> [(&'static str, String); 8] {
    let text = |zh, en| if chinese { zh } else { en };
    [
        (
            text("已连接节点", "Connected peers"),
            peers.connected.to_string(),
        ),
        (
            text("已连接的做种节点", "Connected seeders"),
            peers.seeders.to_string(),
        ),
        (text("正在连接", "Connecting"), peers.connecting.to_string()),
        (text("待连接", "Queued peers"), peers.queued.to_string()),
        (
            text("已发现节点", "Discovered peers"),
            peers.known.to_string(),
        ),
        (
            text("上传速度", "Upload speed"),
            format!("{}/s", format_bytes(peers.upload_bytes_per_second)),
        ),
        (text("已上传", "Uploaded"), format_bytes(peers.uploaded)),
        (text("分享率", "Share ratio"), ratio(peers, downloaded)),
    ]
}
