use crate::kernel_data::with_kernel_data;
use crate::{ExpectedSyscall, SyscallLogEntry};
use libtock_platform::{return_variant, ErrorCode, Register};
use std::convert::TryInto;

// Safety: The arguments must represent a valid Subscribe call as specified by
// TRD 104.
pub(super) unsafe fn subscribe(
    driver_number: Register,
    subscribe_number: Register,
    upcall_fn: Register,
    data: Register,
) -> [Register; 4] {
    let driver_number = driver_number.try_into().expect("Too large driver number");
    let subscribe_number = subscribe_number
        .try_into()
        .expect("Too large subscribe number");
    let (skip_with_error, num_upcalls) = with_kernel_data(|option_kernel_data| {
        let kernel_data = option_kernel_data.expect("Subscribe called but no fake::Kernel exists");

        kernel_data.syscall_log.push(SyscallLogEntry::Subscribe {
            driver_number,
            subscribe_number,
        });

        // Check for an expected syscall. Panics if an expected syscall exists
        // and it does not match this syscall. Otherwise sets skip_with_error to
        // skip_with_error from the expected syscall, or None if none was
        // provided.
        let skip_with_error = match kernel_data.expected_syscalls.pop_front() {
            None => None,
            Some(ExpectedSyscall::Subscribe {
                driver_number: expected_driver_number,
                subscribe_number: expected_subscribe_number,
                skip_with_error,
            }) => {
                assert_eq!(
                    driver_number, expected_driver_number,
                    "expected different driver number"
                );
                assert_eq!(
                    subscribe_number, expected_subscribe_number,
                    "expected different subscribe number"
                );
                skip_with_error
            }
            Some(expected_syscall) => expected_syscall.panic_wrong_call("Subscribe"),
        };

        // Retrieve the number of upcalls for this driver, or None if there is
        // no driver with this number.
        let num_upcalls = kernel_data
            .drivers
            .get(&driver_number)
            .map(|driver_data| driver_data.num_upcalls);

        (skip_with_error, num_upcalls)
    });

    // Convenience function to produce an error return.
    let failure_registers = |error_code: ErrorCode| {
        [
            return_variant::FAILURE_2_U32.into(),
            error_code.into(),
            upcall_fn,
            data,
        ]
    };

    // If skip_with_error was specified, we skip the remainder of this logic and
    // return an error directly.
    if let Some(error_code) = skip_with_error {
        return failure_registers(error_code);
    }

    // Verify the given driver ID was present. If no driver with this ID is
    // present, the kernel returns NOMEM.
    let num_upcalls = match num_upcalls {
        Some(num_upcalls) => num_upcalls,
        None => return failure_registers(ErrorCode::NoMem),
    };

    // If a too-large subscribe number is passed, the kernel returns the Invalid
    // error code.
    if subscribe_number >= num_upcalls {
        return failure_registers(ErrorCode::Invalid);
    }

    // At this point, we know the Subscribe call should succeed.

    let upcall = crate::upcall::Upcall {
        fn_pointer: match upcall_fn.into() {
            0usize => None,
            // Safety: RawSyscalls guarantees that if upcall_fn is not 0, then
            // it is a valid unsafe extern fn(u32, u32, u32, Register). We've
            // already verified upcall_fn is not 0. The niche optimization
            // guarantees that an unsafe extern fn(u32, u32, u32, Register) can
            // be transmuted into an Option<unsafe extern fn(u32, u32, u32,
            // Register)>.
            upcall_fn => unsafe { core::mem::transmute(upcall_fn) },
        },
        data,
    };

    let upcall_id = crate::upcall::UpcallId {
        driver_number,
        subscribe_number,
    };

    // Go back into the kernel data to update the stored upcall and purge the
    // previous upcall from the upcall queue (as required by TRD 104).
    let out_upcall = with_kernel_data(|option_kernel_data| {
        let kernel_data = option_kernel_data.unwrap();
        kernel_data
            .upcall_queue
            .retain(|existing_queue_entry| existing_queue_entry.id != upcall_id);
        kernel_data
            .drivers
            .get_mut(&driver_number)
            .unwrap()
            .upcalls
            .insert(subscribe_number, upcall)
    });

    let out_upcall_fn = out_upcall
        .map_or(0, |out_upcall| match out_upcall.fn_pointer {
            None => 0,
            Some(fn_pointer) => fn_pointer as usize,
        })
        .into();

    let out_upcall_data = out_upcall.map_or(0usize.into(), |out_upcall| out_upcall.data);

    // The Success with 2 U32 variant doesn't specify what is returned in r3. In
    // practice, the kernel will leave that register alone, so we echo data
    // (passed in via r3) out as r3.
    [
        return_variant::SUCCESS_2_U32.into(),
        out_upcall_fn,
        out_upcall_data,
        data,
    ]
}
