/// Print to console
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        #[cfg(not(test))]
        {
            use core::fmt::Write;
            let _ = write!($crate::log::Writer::new(), $($arg)*);
        }
        #[cfg(test)]
        {
            std::print!($($arg)*);
        }
    });
}

/// Print with new line to console
#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => ({
        #[cfg(not(test))]
        {
            use core::fmt::Write;
            let _ = writeln!($crate::log::Writer::new(), $($arg)*);
        }
        #[cfg(test)]
        {
            std::println!($($arg)*);
        }
    });
}

/// Prints an error message.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        println!("{}:ERROR -- {}", core::module_path!(), format_args!($($arg)*));
    };
}

/// Prints a warning message.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        println!("{}:WARN -- {}", core::module_path!(), format_args!($($arg)*));
    };
}

/// Prints an info message.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        println!("{}:INFO -- {}", core::module_path!(), format_args!($($arg)*));
    };
}

/// Prints a debug message.
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if cfg!(any(target_arch = "aarch64", target_arch = "riscv64")) {
            println!("{}:DEBUG -- {}", core::module_path!(), format_args!($($arg)*));
        }
    };
}

/// Prints a trace message.
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        if false {
            println!("{}:TRACE -- {}", core::module_path!(), format_args!($($arg)*));
        }
    };
}
