// How do we do per-cpu data in rust.
// There are two popular ways to do per CPU data. In NT, there is a CPU structure which the world and his uncle
// understands. Everything touches it.
// In Linux instead there is a per-cpu variable mechanism. I like the sound of a per-cpu variable mechanism a lot.
// It means that systems that care about the CPU do not need to all route through a single structure.

// So, how an we do that.

// Our ultimate aim is to have something that implements Deref and DerefMut. So we're probably looking at something
// similar to how lazy-statics or thread-local are implemented.

// Thread local is an interesting pattern to follow. It has a "key" object that has a with
// method on it that gets you a reference to the object and makes sure it is initialized.
// That might work. So under the hood, where will the storage live?

// One interesting problem is that I'm expecting to need to be able to use this mechanism
// for CPU 0 before I can allocate memory. So how do I do that?

// Going to need some unsafe code for this. We don't need to be particularly
// thread safe.
use core::cell::{Cell, UnsafeCell};
use core::mem::MaybeUninit;
use crate::types::VirtualAddress;

extern "C" {
    static __kernel_per_cpu_start: u8;
    static __kernel_per_cpu_end: u8;
}

fn get_per_cpu_start() -> VirtualAddress {
    VirtualAddress::from_ptr(unsafe { &__kernel_per_cpu_start })
}

fn get_per_cpu_end() -> VirtualAddress {
    VirtualAddress::from_ptr(unsafe { &__kernel_per_cpu_end })
}

pub struct PerCpuPayload<T> {
    state: Cell<usize>,
    data: UnsafeCell<MaybeUninit<T>>,
}

const NOT_INITIALIZED: usize = 0;
const COMPLETE: usize = 1;
const PANICKED: usize = 2;

const BIGSPACE_SIZE: usize = 1024;

#[repr(align(4096))]
#[repr(C)]
struct BigSpace {
    buf: MaybeUninit<[u8;BIGSPACE_SIZE]>,
}

static mut big_space: BigSpace = BigSpace { buf: MaybeUninit::uninit() };

fn get_per_cpu_base() -> VirtualAddress {
    assert!(get_per_cpu_end() - get_per_cpu_start() < BIGSPACE_SIZE as u64);
    unsafe { VirtualAddress::from_ptr(&big_space) }
}

impl<T> PerCpuPayload<T> {
    pub const INIT: Self = PerCpuPayload { state: Cell::new(NOT_INITIALIZED), data: UnsafeCell::new(MaybeUninit::uninit()) };

    pub const fn new() -> Self {
        Self::INIT
    }

    fn offset_of(payload: &Self) -> usize {
        (VirtualAddress::from_ptr(&*payload) - get_per_cpu_start()) as usize
    }

    pub fn get_cpu_payload(payload: &Self) -> &Self {
        let addr = get_per_cpu_base() + Self::offset_of(payload) as u64;
        let ret: &Self = unsafe { &*addr.as_ptr() };
        ret
    }

    fn force_get<'a>(&'a self) -> &'a T {
        unsafe {
            // SAFETY: 
            // * `UnsafeCell`/inner deref: data never changes again
            // * `MaybeUninit`/outer deref: data was initialized
            &*(*self.data.get()).as_ptr()
        }
    }

    pub fn get<'a>(&'a self, builder: impl FnOnce() -> T) -> &'a T {
        match self.state.get() {
            NOT_INITIALIZED => {
                let mut finish = Finish { state: &self.state, panicked: true };
                unsafe {
                    (*self.data.get()).as_mut_ptr().write(builder())
                };
                finish.panicked = false;

                self.state.set(COMPLETE);
                self.force_get()
            },
            COMPLETE => {
                self.force_get()
            },
            PANICKED => panic!("Per cpu initializer panicked"),
            _ => unsafe { core::hint::unreachable_unchecked() },
        }
    }
}

struct Finish<'a> {
    state: &'a Cell<usize>,
    panicked: bool,
}

impl<'a> Drop for Finish<'a> {
    fn drop(&mut self) {
        if self.panicked {
            self.state.set(PANICKED);
        }
    }
}

unsafe impl<T: Send + Sync> Sync for PerCpuPayload<T> { }
unsafe impl<T: Send> Send for PerCpuPayload<T> { }

#[macro_export]
macro_rules! per_cpu {
    // empty (base case for the recursion)
    () => {};

    // process multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => (
        $crate::__per_cpu_inner!($(#[$attr])* $vis $name, $t, $init);
        $crate::per_cpu!($($rest)*);
    );

    // handle a single declaration
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        $crate::__per_cpu_inner!($(#[$attr])* $vis $name, $t, $init);
    );
}

#[macro_export]
#[doc(hidden)]
macro_rules! __per_cpu_inner {
    (@make $($attr:meta)* $vis:vis, $name:ident : $t:ty) => {
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        $(#[$attr])*
        $vis struct $name { payload: $crate::percpu::PerCpuPayload<$t> }
        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        #[link_section = ".data..percpu"]
        $vis static $name: $name = $name { payload: $crate::percpu::PerCpuPayload::INIT };
    };
    (@tail $name:ident : $t:ty = $init:expr) => {
        impl core::ops::Deref for $name {
            type Target = $t;
            fn deref(&self) -> &Self::Target {
                #[inline(always)]
                fn __static_ref_initialize() -> $t { $init }

                let payload = $crate::percpu::PerCpuPayload::get_cpu_payload(&self.payload);
                payload.get(__static_ref_initialize)
            }
        }

        impl $crate::percpu::PerCpu for $name {
            fn initialize(percpu: &Self) {
                let _ = &**percpu;
            }
        }
    };

    ($(#[$attr:meta])* $vis:vis $name:ident, $t:ty, $init:expr) => {
        $crate::__per_cpu_inner!(@make $($attr)* $vis, $name : $t);
        $crate::__per_cpu_inner!(@tail $name: $t = $init);
    };
}

pub trait PerCpu {
    #[doc(hidden)]
    fn initialize(percpu: &Self);
}

pub fn initialize(percpu: &impl PerCpu) {
    PerCpu::initialize(percpu)
}

per_cpu! {
    static cheese: [u32;10] = [0;10];
    static tits: u8 = 17;
}

pub fn whats_going_on() {
    panic!("hello {}", *tits);
}