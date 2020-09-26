#![no_std]
#![cfg_attr(test, no_main)]
#![feature(abi_x86_interrupt)]
#![feature(asm)]
#![feature(box_syntax)]
#![feature(alloc_error_handler)]
#![feature(const_fn)]
#![feature(const_in_array_repeat_expressions)]
#![feature(const_panic)]
#![feature(core_intrinsics)]
#![feature(custom_test_frameworks)]
#![feature(global_asm)]
#![feature(maybe_uninit_extra)]
#![feature(never_type)]
#![feature(slice_fill)]
#![feature(step_trait)]
#![feature(step_trait_ext)]
#![feature(thread_local)]
#![feature(try_blocks)]
#![feature(unsafe_cell_raw_get)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate rlibc;

//#[macro_use]
extern crate alloc;

pub mod acpi;
pub mod allocator;
pub mod devices;
pub mod gdt;
pub mod idt;
pub mod init;
pub mod init_mutex;
pub mod interrupts;
pub mod io_port;
pub mod ipi;
pub mod mm;
pub mod paging;
pub mod physmem;
pub mod serial;
pub mod vga_buffer;

#[cfg(test)]
use bootloader::BootInfo;
use core::panic::PanicInfo;

#[global_allocator]
static ALLOCATOR: allocator::Allocator = allocator::Allocator;

#[alloc_error_handler]
fn alloc_error_handler(layout: alloc::alloc::Layout) -> ! {
    panic!("allocation error: {:?}", layout);
}

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    loop {}
}

#[cfg(test)]
fn idle_loop() -> ! {
    unsafe {
        *(0xdeadbeef as *mut u64) = 42;
    };

    todo!("BIG IDLE: This would be the idle loop")
}

#[cfg(test)]
fn run_tests() -> ! {
    test_main();
    idle_loop();
}

/// Entry point for `cargo test`
#[cfg(test)]
#[no_mangle]
pub unsafe extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    init::kstart(boot_info, run_tests)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}
