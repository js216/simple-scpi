# simple-scpi

A lightweight SCPI (Standard Commands for Programmable Instruments) command
parser for Rust.

Define your instrument's command set as a table of pattern strings and handler
functions, and the library takes care of parsing input lines — including
short/long keyword forms, numeric suffixes, optional nodes, and compound
commands.

## Usage

```rust
use simple_scpi::{CommandSet, Command, Handler, Param};

let set = CommandSet::from_table(&[
    ("*IDN?",                       handle_idn  as Handler),
    ("SOURce#:FREQuency num",       handle_freq as Handler),
    ("SOURce#:VOLTage[:LEVel] num", handle_volt as Handler),
    ("OUTPut#:STATe bool",          handle_outp as Handler),
]).unwrap();

for cmd in set.parse("SOUR2:FREQ 1e6;OUTP1:STAT ON").unwrap() {
    set.dispatch(&cmd);
}
```

Each handler receives a `&Command` with parsed `params` and numeric
`suffixes`:

```rust
fn handle_freq(cmd: &Command) {
    let Param::Numeric(f) = cmd.params[0] else { return };
    println!("ch{} freq = {f}", cmd.suffixes[0]);
}
```

See [`examples/demo.rs`](examples/demo.rs) for a complete working example.

## Pattern syntax

| Syntax | Meaning |
|--------|---------|
| `SYSTem` | Mixed-case keyword — uppercase is the required short form, full word is the long form. Matches `SYST`, `SYSTE`, `SYSTEM` (case-insensitive). |
| `:` | Keyword hierarchy separator. |
| `#` | Numeric suffix placeholder (defaults to 1 when omitted by the user). |
| `[...]` | Optional keyword node, e.g. `[:LEVel]`. |
| `?` | Query suffix. |
| `num` | Numeric parameter (integers, floats, scientific notation, `#H`/`#Q`/`#B` literals, optional unit suffix). |
| `bool` | Boolean parameter (`ON`/`OFF`/`1`/`0`). |
| `str` | String parameter (quoted or unquoted). |

Multiple commands on one line are separated by `;`.
