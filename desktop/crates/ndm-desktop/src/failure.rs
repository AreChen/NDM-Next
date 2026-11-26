use crate::preferences::Language;

pub(crate) fn description(code: &str, language: Language) -> &str {
    match code {
        "download SHA-256 does not match the source" => language.text(
            "文件的 SHA-256 与来源提供的摘要不一致，校验失败。",
            "download SHA-256 does not match the source",
        ),
        "dash_drm_unsupported" => language.text(
            "此 DASH 媒体受 DRM 保护，无法下载。",
            "DRM-protected DASH media cannot be downloaded.",
        ),
        "dash_dynamic_unsupported" => language.text(
            "暂不支持动态 DASH 直播，可下载静态 DASH 和 HLS 直播。",
            "Dynamic DASH is unsupported; static DASH and HLS live streams are supported.",
        ),
        "dash_layout_unsupported" => language.text(
            "暂不支持此 DASH 清单或媒体格式。",
            "This DASH manifest or media format is unsupported.",
        ),
        "dash_recovery_conflict" => language.text(
            "媒体与已保存内容不一致，请更换有效下载链接后重新下载。",
            "Saved media no longer matches; replace the download URL to start again.",
        ),
        "media_tool_unavailable" => language.text(
            "本地媒体工具缺失，请重新安装程序。",
            "Local media tools are missing; reinstall the application.",
        ),
        "dash_failed" => language.text(
            "DASH 下载或封装失败，请检查链接与媒体格式。",
            "DASH download or remux failed; check the URL and media format.",
        ),
        "plugin_postprocessing_failed" => language.text(
            "下载插件未能完成媒体合并或封装。",
            "The download plugin could not merge or finalize the media.",
        ),
        "plugin_options_invalid" => language.text(
            "插件选项无效，请检查插件设置。",
            "The plugin options are invalid. Check the plugin settings.",
        ),
        "download plugin network error" => language.text(
            "下载插件网络连接失败，请检查网络或代理设置。",
            "download plugin network error",
        ),
        "download plugin storage error" => language.text(
            "下载插件无法保存文件，请检查磁盘空间和目录权限。",
            "download plugin storage error",
        ),
        "download plugin authentication failed" => language.text(
            "下载插件身份验证失败。",
            "download plugin authentication failed",
        ),
        "download plugin does not support this source" => language.text(
            "下载插件不支持此链接或文件。",
            "download plugin does not support this source",
        ),
        "download plugin failed" => language.text("下载插件执行失败。", "download plugin failed"),
        _ => code,
    }
}
