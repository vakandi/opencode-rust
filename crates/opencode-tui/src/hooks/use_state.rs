use std::cell::RefCell;
use std::rc::Rc;

pub struct UseState<T> {
    value: Rc<RefCell<T>>,
    changed: Rc<RefCell<bool>>,
}

impl<T: Clone> UseState<T> {
    pub fn new(initial: T) -> Self {
        Self {
            value: Rc::new(RefCell::new(initial)),
            changed: Rc::new(RefCell::new(false)),
        }
    }

    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    pub fn set(&self, value: T) {
        *self.value.borrow_mut() = value;
        *self.changed.borrow_mut() = true;
    }

    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        f(&mut self.value.borrow_mut());
        *self.changed.borrow_mut() = true;
    }

    pub fn changed(&self) -> bool {
        *self.changed.borrow()
    }

    pub fn clear_changed(&self) {
        *self.changed.borrow_mut() = false;
    }
}

impl<T: Clone> Clone for UseState<T> {
    fn clone(&self) -> Self {
        Self {
            value: Rc::clone(&self.value),
            changed: Rc::clone(&self.changed),
        }
    }
}
