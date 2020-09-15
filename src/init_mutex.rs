use core::ops::{Deref, DerefMut};
use spin::{Mutex, MutexGuard};

pub struct InitMutex<T> {
    lock: Mutex<Option<T>>,
}

impl<T> InitMutex<T> {
    pub const fn new() -> Self {
        Self {
            lock: Mutex::new(None),
        }
    }

    pub fn init(&self, t: T) {
        *self.lock.lock() = Some(t);
    }

    pub fn lock<'a>(&'a self) -> InitMutexGuard<'a, T> {
        InitMutexGuard {
            guard: self.lock.lock(),
        }
    }
}

pub struct InitMutexGuard<'a, T> {
    guard: MutexGuard<'a, Option<T>>,
}

impl<'a, T> Deref for InitMutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("InitMutexGuard has not been initialized")
    }
}

impl<'a, T> DerefMut for InitMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("InitMutexGuard has not been initialized")
    }
}
