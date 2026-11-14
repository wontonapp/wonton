use super::*;
use wonton_protocol::TabPage;

#[derive(Clone)]
pub(super) struct ContentPage {
    pub(super) stored: StoredItem,
    pub(super) references: usize,
    pub(super) view: watch::Sender<TabPage>,
}

/// 独立内容页的存活引用；页面状态与主栈共享，但不改变主栈。
#[derive(uniffi::Object)]
pub struct ContentPageSubscription {
    link: Arc<PagesLink>,
    tab: TabRef,
    receiver: tokio::sync::Mutex<watch::Receiver<TabPage>>,
    current: watch::Receiver<TabPage>,
    cancelled: watch::Sender<bool>,
}

#[uniffi::export]
impl ContentPageSubscription {
    pub fn page(&self) -> TabPage {
        self.current.borrow().clone()
    }

    pub async fn next(&self) -> Option<TabPage> {
        let mut cancelled = self.cancelled.subscribe();
        let mut receiver = self.receiver.lock().await;
        tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => None,
            changed = receiver.changed() => {
                changed.ok()?;
                Some(receiver.borrow_and_update().clone())
            }
        }
    }

    pub fn cancel(&self) {
        if !self.cancelled.send_replace(true) {
            self.link.release_content(&self.tab);
        }
    }
}

impl Drop for ContentPageSubscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl PagesLink {
    pub(crate) fn subscribe_content_page(
        self: &Arc<Self>,
        tab: TabRef,
    ) -> Result<Arc<ContentPageSubscription>, CoreError> {
        let mut receiver = None;
        self.runtime_change(|state, plugins| {
            if let Some(content) = state.content.get_mut(&tab) {
                content.references += 1;
                receiver = Some(content.view.subscribe());
                return Ok(());
            }
            let stored = match stored_item_for_tab(&tab, plugins) {
                Ok(stored) => stored,
                Err(error @ CoreError::NotReady { .. }) => {
                    stored_item_from_config(&state.config, &tab).ok_or(error)?
                }
                Err(error) => return Err(error),
            };
            let page = build_page_item(state, &stored, plugins);
            let view = watch::Sender::new(page);
            receiver = Some(view.subscribe());
            state.content.insert(
                tab.clone(),
                ContentPage {
                    stored,
                    references: 1,
                    view,
                },
            );
            Ok(())
        })?;
        let mut receiver = receiver.expect("successful subscription creates a receiver");
        receiver.mark_changed();
        Ok(Arc::new(ContentPageSubscription {
            link: self.clone(),
            tab,
            current: receiver.clone(),
            receiver: tokio::sync::Mutex::new(receiver),
            cancelled: watch::Sender::new(false),
        }))
    }

    fn release_content(&self, tab: &TabRef) {
        // Releasing a lease cannot fail and must not depend on configuration persistence.
        let _writing = lock(&self.writing);
        let mut state = lock(&self.state);
        if let Some(content) = state.content.get_mut(tab) {
            content.references -= 1;
            if content.references == 0 {
                state.content.remove(tab);
            }
        }
        clear_gone_prompt(&mut state);
        let plugins = self.plugins_view.borrow().clone();
        let view = make_view(&state, &plugins, self.sync_problem.borrow().is_some());
        drop(state);
        self.publish(view, &plugins);
    }

    pub(super) fn content_page(&self, tab: &TabRef) -> Option<TabPage> {
        lock(&self.state)
            .content
            .get(tab)
            .map(|content| content.view.borrow().clone())
    }
}
