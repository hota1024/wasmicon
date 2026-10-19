//! Limits chosen by the port. As with the arena, the core does not decide them itself.

/// Runtime limits.
#[derive(Clone, Copy)]
pub struct Config {
    /// Maximum depth of the value stack during validation.
    pub max_value_stack: usize,
    /// Maximum nesting depth of control structures.
    pub max_control_depth: usize,
    /// Maximum number of locals (parameters included) in one function.
    pub max_locals: usize,
    /// Maximum number of linear memory pages (1 page = 64 KiB).
    pub max_memory_pages: u32,
    /// Maximum number of table elements.
    pub max_table_elems: u32,
    /// Maximum call depth at run time. Exceeding it returns `Exhausted`.
    pub max_call_depth: usize,
    /// Number of operand stack slots at run time.
    pub operand_stack_slots: usize,
}

impl Config {
    /// Defaults for a host PC. Microcontroller ports use smaller values.
    ///
    /// This is a `const` separate from `Default` because the board profiles
    /// (`profile` in `ports/common`) use it in a const context. Writing the
    /// values twice would let them drift apart.
    pub const DEFAULT: Config = Config {
        max_value_stack: 8192,
        max_control_depth: 2048,
        max_locals: 8192,
        max_memory_pages: 65536,
        max_table_elems: 0x0010_0000,
        max_call_depth: 512,
        operand_stack_slots: 16384,
    };
}

impl Default for Config {
    fn default() -> Self {
        Config::DEFAULT
    }
}
