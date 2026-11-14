pub(crate) fn description(code: &str, chinese: bool) -> &str {
    match (code, chinese) {
        ("dash_drm_unsupported", true) => "此 DASH 媒体受 DRM 保护，无法下载。",
        ("dash_drm_unsupported", false) => "DRM-protected DASH media cannot be downloaded.",
        ("dash_dynamic_unsupported", true) => {
            "暂不支持动态 DASH 直播，可下载静态 DASH 和 HLS 直播。"
        }
        ("dash_dynamic_unsupported", false) => {
            "Dynamic DASH is unsupported; static DASH and HLS live streams are supported."
        }
        ("dash_layout_unsupported", true) => "暂不支持此 DASH 清单或媒体格式。",
        ("dash_layout_unsupported", false) => "This DASH manifest or media format is unsupported.",
        ("dash_recovery_conflict", true) => {
            "媒体与已保存内容不一致，请更换有效下载链接后重新下载。"
        }
        ("dash_recovery_conflict", false) => {
            "Saved media no longer matches; replace the download URL to start again."
        }
        ("media_tool_unavailable", true) => "本地媒体工具缺失，请重新安装程序。",
        ("media_tool_unavailable", false) => {
            "Local media tools are missing; reinstall the application."
        }
        ("dash_failed", true) => "DASH 下载或封装失败，请检查链接与媒体格式。",
        ("dash_failed", false) => "DASH download or remux failed; check the URL and media format.",
        _ => code,
    }
}
