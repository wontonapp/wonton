use std::cell::RefCell;

type Hook = Box<dyn FnOnce()>;

thread_local! {
    static AFTER_INITIAL_READ: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

pub(in crate::gateway) fn install(hook: impl FnOnce() + 'static) {
    AFTER_INITIAL_READ.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

pub(super) fn after_initial_read() {
    let hook = AFTER_INITIAL_READ.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
