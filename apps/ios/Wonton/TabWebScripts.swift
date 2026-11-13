/// 注进页面的脚本：媒体通道、缩放、字号（从 TabWebSession 拆出，按职责分文件）。
extension TabWebSession {
    /// 核心的媒体脚本经 window.wontonMedia 发状态（同 Android 的 WebMessageListener 名字）；
    /// iOS 的消息处理挂在 window.webkit.messageHandlers 上，先垫一段把两边对上。
    private static let mediaChannelScript = """
    window.wontonMedia = {
      postMessage: (message) => window.webkit.messageHandlers.wontonMedia.postMessage(message),
    };
    """

    /// WebKit 在输入框字号小于 16px 时聚焦会放大页面，失焦不缩回（#442）。聚焦放大的上限
    /// 取 viewport 的 maximum-scale，不受 ignoresViewportScaleLimits 影响，所以补上
    /// maximum-scale=1 只拦它。页面自己写了 maximum-scale 的照它的；没有 viewport 的
    /// 是桌面宽度的页面，聚焦放大对它有用，不动。
    private static let focusZoomScript = """
        (() => {
          const meta = document.querySelector('meta[name="viewport"]');
          if (!meta || /maximum-scale/i.test(meta.content)) return;
          meta.content = meta.content + ', maximum-scale=1';
        })();
        """

    private func textSizeScript(_ scale: CGFloat) -> String {
        """
        (() => {
          const apply = () => {
            if (!document.documentElement) return false;
            document.documentElement.style.setProperty(
              '-webkit-text-size-adjust', '\(Double(scale * 100))%', 'important'
            );
            return true;
          };
          if (!apply()) {
            const observer = new MutationObserver(() => {
              if (apply()) observer.disconnect();
            });
            observer.observe(document, { childList: true });
          }
        })();
        """
    }
}
