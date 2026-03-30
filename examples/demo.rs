use simple_scpi::{Command, CommandSet, Handler, Param};

const TABLE: &[(&str, Handler)] = &[
    ("*IDN?", idn),
    ("*RST", rst),
    ("SOURce#:FREQuency num", src_freq),
    ("SOURce#:FREQuency?", src_freq_q),
    ("SOURce#:VOLTage[:LEVel] num", src_volt),
    ("OUTPut#:STATe bool", outp_stat),
    ("DISPlay:TEXT str", disp_text),
];

fn main() {
    let set = CommandSet::from_table(TABLE).expect("bad command table");
    let stdin = std::io::stdin();
    let mut line = String::new();
    while stdin.read_line(&mut line).is_ok_and(|n| n > 0) {
        match set.parse(line.trim()) {
            Ok(cmds) => {
                for cmd in &cmds {
                    set.dispatch(cmd);
                }
            }
            Err(e) => eprintln!("ERROR: {e}"),
        }
        line.clear();
    }
}

fn idn(_: &Command) {
    println!("ACME Instruments,Model 100,SN0001,1.0");
}

fn rst(_: &Command) {
    println!("reset");
}

fn src_freq(cmd: &Command) {
    let Param::Numeric(f) = cmd.params[0] else { return };
    println!("source {} freq = {f} Hz", cmd.suffixes[0]);
}

fn src_freq_q(cmd: &Command) {
    println!("source {} freq?", cmd.suffixes[0]);
}

fn src_volt(cmd: &Command) {
    let Param::Numeric(v) = cmd.params[0] else { return };
    println!("source {} volt = {v} V", cmd.suffixes[0]);
}

fn outp_stat(cmd: &Command) {
    let Param::Bool(b) = cmd.params[0] else { return };
    println!("output {} = {}", cmd.suffixes[0], if b { "ON" } else { "OFF" });
}

fn disp_text(cmd: &Command) {
    let Param::String(ref s) = cmd.params[0] else { return };
    println!("display: {s:?}");
}
