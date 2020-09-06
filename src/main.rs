#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(rust_kern::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate rust_kern;
