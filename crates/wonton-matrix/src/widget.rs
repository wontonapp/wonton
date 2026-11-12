//! 插件网页作为 Matrix widget 在房间里跑（ADR-0096）：起 matrix-sdk 的 widget 驱动，地址里的占位符由它替换，
//! 能力按“页面请求 ∩ 清单声明 ∩ 核心允许”批（[`grants`]）。驱动只认打开它的这个房间。

mod grants;

use std::future::Future;
use std::pin::Pin;

use matrix_sdk::RoomState;
use matrix_sdk::ruma::{RoomId, TransactionId};
use matrix_sdk::widget::{ClientProperties, WidgetDriver, WidgetSettings};

pub use matrix_sdk::widget::WidgetDriverHandle;

use crate::{Error, MatrixSession, Result};

/// 打开一个 widget 要的输入。
pub struct WidgetRequest {
    /// 页面的完整地址，带 widget 规范的占位符（`$matrix_widget_id` 等），Wonton 自己的占位符已经替换过。
    pub url: String,
    /// 清单声明的能力串：批给页面的不会超出这些。
    pub declared: Vec<String>,
    /// App 的语言（BCP 47），填 `$org.matrix.msc2873.client_language`。
    pub language: String,
}

/// 起好的 widget：页面地址、和页面互发消息的句柄，以及要一直跑着的驱动。
pub struct OpenedWidget {
    pub url: String,
    pub handle: WidgetDriverHandle,
    /// 驱动本身：页面关掉时调用方丢掉它，这个 widget 会话就结束了。
    pub driver: Pin<Box<dyn Future<Output = ()> + Send>>,
}

/// 页面看到的客户端 ID（`$org.matrix.msc2873.client_id`）。
const CLIENT_ID: &str = "org.wonton";

impl MatrixSession {
    /// 在 `room_id` 里为页面起一个 widget 驱动。每次都是新的 widget ID；页面加载完、发来 `content_loaded`
    /// 后驱动才开始协商能力（`init_on_content_load`）。
    pub async fn open_widget(
        &self,
        room_id: &RoomId,
        request: WidgetRequest,
    ) -> Result<OpenedWidget> {
        let room = self.room(room_id)?;
        if room.state() != RoomState::Joined {
            return Err(Error::NotReady(
                "还没加入这个聊天，加入后才能在里面打开插件".to_owned(),
            ));
        }
        let widget_id = format!("wonton-{}", TransactionId::new());
        let settings = WidgetSettings::new(widget_id, true, &request.url)
            .map_err(|error| Error::InvalidInput(format!("widget 地址不对：{error}")))?;
        // 语言来自 App 的语言表，都是合法的 BCP 47；万一解析不了，SDK 用 en-US。
        let language = request.language.parse::<language_tags::LanguageTag>().ok();
        // 明暗由壳按系统和 App 设置合成，起驱动时核心不知道：`$org.matrix.msc2873.client_theme` 由 SDK 填 `light`，
        // 页面的明暗看桥注入的 `color-scheme`（ADR-0096）。
        let properties = ClientProperties::new(CLIENT_ID, language, None);
        let url = settings
            .generate_webview_url(&room, properties)
            .await
            .map_err(|error| Error::InvalidInput(format!("widget 地址不对：{error}")))?;
        let (driver, handle) = WidgetDriver::new(settings);
        let provider = grants::Grants::new(request.declared, self.user_id().to_owned());
        let driver = Box::pin(async move {
            // 只在页面断开或驱动出了终止性错误时返回，SDK 不给原因。
            if driver.run(room, provider).await.is_err() {
                tracing::info!("widget driver stopped");
            }
        });
        Ok(OpenedWidget {
            url: url.to_string(),
            handle,
            driver,
        })
    }
}
