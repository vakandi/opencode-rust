use std::cell::RefCell;
use std::rc::Rc;

type CleanupFn = Box<dyn FnMut()>;

pub struct UseEffect {
    cleanup: Rc<RefCell<Option<CleanupFn>>>,
}

impl UseEffect {
    pub fn new() -> Self {
        Self {
            cleanup: Rc::new(RefCell::new(None)),
        }
    }

    pub fn run<F>(&self, effect: F)
    where
        F: FnOnce() -> Option<Box<dyn FnMut()>>,
    {
        if let Some(mut cleanup) = self.cleanup.borrow_mut().take() {
            cleanup();
        }

        if let Some(new_cleanup) = effect() {
            *self.cleanup.borrow_mut() = Some(new_cleanup);
        }
    }
}

impl Default for UseEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for UseEffect {
    fn drop(&mut self) {
        if let Some(mut cleanup) = self.cleanup.borrow_mut().take() {
            cleanup();
        }
    }
}
