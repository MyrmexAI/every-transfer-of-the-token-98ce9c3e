#![no_std]

pub mod error;
pub mod processor;
pub mod state;
pub mod token;

use pinocchio::{no_allocator, nostd_panic_handler, program_entrypoint};

pinocchio::address::declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

program_entrypoint!(processor::process_instruction);
no_allocator!();
nostd_panic_handler!();
