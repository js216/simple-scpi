/// A lightweight SCPI command parser.
///
/// Pattern syntax (used in the table passed to [`CommandSet::from_table`]):
///
/// - Keywords use mixed case: uppercase = required, lowercase = optional.
///   `SYSTem` matches `SYST`, `SYSTE`, `SYSTEM` (all case-insensitive).
/// - `:` separates hierarchical keywords.
/// - `#` in a keyword means a numeric suffix (defaults to 1 when absent).
/// - `[...]` encloses an optional keyword node, including its leading colon
///   when written as `[:NODE]`.
/// - `?` at the end marks a query.
/// - After keywords, a space-separated token declares the parameter type:
///   `num`, `bool`, `str`.
///
/// Input lines may contain multiple commands separated by `;`.
/// A leading `:` (or `:` after `;`) resets to the root of the command tree;
/// without it, compound commands continue from the previous tree position.

use std::fmt;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A parsed parameter value.
#[derive(Debug, Clone)]
pub enum Param {
    Numeric(f64),
    Bool(bool),
    String(String),
}

/// A single parsed command ready for dispatch.
#[derive(Debug)]
pub struct Command {
    /// Index into the table that was passed to [`CommandSet::from_table`].
    pub index: usize,
    /// Parameter values extracted from the input.
    pub params: Vec<Param>,
    /// Numeric suffix values (`#` placeholders), in order of appearance.
    /// Defaults to `1` when the user omits the suffix digit.
    pub suffixes: Vec<u32>,
}

/// Handler function signature.
pub type Handler = fn(&Command);

/// Compiled command set built from a table of `(pattern, handler)` pairs.
pub struct CommandSet {
    entries: Vec<Entry>,
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum ParamKind {
    Num,
    Bool,
    Str,
}

/// One segment of a compiled keyword pattern.
#[derive(Debug, Clone)]
enum Segment {
    /// A plain keyword with short + long forms (both stored uppercase).
    Keyword { short: String, long: String },
    /// A `#` numeric suffix placeholder.
    NumericSuffix,
}

/// A compiled command entry.
struct Entry {
    /// Segments that must be matched (flattened; optional groups are expanded).
    segments: Vec<Segment>,
    /// Whether any segment group was optional (generates alternate accept sets).
    optional_groups: Vec<OptGroup>,
    is_query: bool,
    param: Option<ParamKind>,
    handler: Handler,
}

/// Represents one `[...]` optional group by the range of segment indices it
/// covers.  When the group is skipped, matching jumps from `start` to `end`.
#[derive(Debug, Clone)]
struct OptGroup {
    start: usize,
    end: usize, // exclusive
}

#[derive(Debug)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Pattern compiler
// ---------------------------------------------------------------------------

/// Extract the short form (uppercase chars) and the long form (all chars)
/// from a mixed-case keyword like `SYSTem`.
fn short_long(token: &str) -> (String, String) {
    let long = token.to_ascii_uppercase();
    // Short form = leading uppercase characters.
    let short: String = token
        .chars()
        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '+' || *c == '-')
        .collect::<String>()
        .to_ascii_uppercase();
    // If the token is all-uppercase already (like `*IDN` or `BAUD`), short == long.
    (short, long)
}

// ---------------------------------------------------------------------------
// Fix: parse_one_keyword needs to emit NumericSuffix after a keyword that
// has `#` glued to it.  Refactor compile_pattern to handle this.
// ---------------------------------------------------------------------------

/// Revised: compile one keyword-and-maybe-suffix, returning 1 or 2 segments.
fn parse_keyword_segments(chars: &[char], pos: &mut usize) -> Vec<Segment> {
    if chars[*pos] == '#' {
        *pos += 1;
        return vec![Segment::NumericSuffix];
    }

    let start = *pos;
    while *pos < chars.len() && !matches!(chars[*pos], ':' | '[' | ']' | '#') {
        *pos += 1;
    }
    let token: String = chars[start..*pos].iter().collect();
    let (short, long) = short_long(&token);
    let kw = Segment::Keyword {
        short: short.to_ascii_uppercase(),
        long: long.to_ascii_uppercase(),
    };

    if *pos < chars.len() && chars[*pos] == '#' {
        *pos += 1;
        vec![kw, Segment::NumericSuffix]
    } else {
        vec![kw]
    }
}

/// Compile pattern (revised, using `parse_keyword_segments`).
fn compile(
    pattern: &str,
) -> Result<(Vec<Segment>, Vec<OptGroup>, bool, Option<ParamKind>), ParseError> {
    let (kw_part, param) = if pattern.ends_with(" num") {
        (&pattern[..pattern.len() - 4], Some(ParamKind::Num))
    } else if pattern.ends_with(" bool") {
        (&pattern[..pattern.len() - 5], Some(ParamKind::Bool))
    } else if pattern.ends_with(" str") {
        (&pattern[..pattern.len() - 4], Some(ParamKind::Str))
    } else {
        (pattern, None)
    };

    let is_query = kw_part.ends_with('?');
    let kw_part = if is_query {
        &kw_part[..kw_part.len() - 1]
    } else {
        kw_part
    };

    let mut segments: Vec<Segment> = Vec::new();
    let mut opt_groups: Vec<OptGroup> = Vec::new();

    let chars: Vec<char> = kw_part.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == ':' {
            i += 1;
            continue;
        }

        if chars[i] == '[' {
            let group_start = segments.len();
            i += 1;
            if i < chars.len() && chars[i] == ':' {
                i += 1;
            }
            while i < chars.len() && chars[i] != ']' {
                if chars[i] == ':' {
                    i += 1;
                    continue;
                }
                let segs = parse_keyword_segments(&chars, &mut i);
                segments.extend(segs);
            }
            if i < chars.len() && chars[i] == ']' {
                i += 1;
            }
            opt_groups.push(OptGroup {
                start: group_start,
                end: segments.len(),
            });
        } else {
            let segs = parse_keyword_segments(&chars, &mut i);
            segments.extend(segs);
        }
    }

    Ok((segments, opt_groups, is_query, param))
}

// ---------------------------------------------------------------------------
// Keyword matching
// ---------------------------------------------------------------------------

/// Check if `input` (already uppercased) matches a keyword with the given
/// short and long forms.  Returns `true` if input length is between
/// short.len() and long.len() (inclusive) and is a prefix of `long` that
/// contains at least `short`.
fn keyword_matches(short: &str, long: &str, input: &str) -> bool {
    let ilen = input.len();
    if ilen < short.len() || ilen > long.len() {
        return false;
    }
    // input must be a prefix of long of length >= short.len()
    long[..ilen] == *input
}

// ---------------------------------------------------------------------------
// Input tokeniser
// ---------------------------------------------------------------------------

/// Split a single command string (no `;`) into keyword tokens and a parameter
/// portion.  Returns `(keyword_tokens, param_str)`.
///
/// Keywords are split on `:`.  The text after the last keyword containing a
/// space (or after keywords if all of them are pure keywords) is the param
/// string.
fn tokenise_command(input: &str) -> (Vec<String>, Option<String>) {
    let input = input.trim();
    if input.is_empty() {
        return (vec![], None);
    }

    // For common commands (*IDN?, *RST, etc.) there's no colon hierarchy.
    // Find where keywords end and parameters begin.
    // Parameters start after a space that is *not* inside quotes.
    let bytes = input.as_bytes();
    let mut kw_end = input.len();
    // Find the first space or quote that separates keywords from params.
    for (j, &b) in bytes.iter().enumerate() {
        if b == b'\'' || b == b'"' {
            // Quote starts the parameter portion.
            kw_end = j;
            break;
        }
        if b == b' ' || b == b'\t' {
            kw_end = j;
            break;
        }
    }

    let kw_str = &input[..kw_end];
    let param_str = input[kw_end..].trim();
    let param = if param_str.is_empty() {
        None
    } else {
        Some(param_str.to_string())
    };

    // Split keywords on `:`, preserving `?` as part of the last token.
    // Also keep `*` prefix glued to the token.
    let tokens: Vec<String> = if kw_str.starts_with('*') {
        // Common command: single token.
        vec![kw_str.to_ascii_uppercase()]
    } else {
        kw_str
            .split(':')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_uppercase())
            .collect()
    };

    (tokens, param)
}

/// Attempt to extract a numeric suffix from the end of a token.
/// E.g. `"SOURCE2"` with keyword long `"SOURCE"` -> Some(2), remaining = `"SOURCE"`.
/// Returns `(token_without_suffix, suffix_value)`.
fn extract_suffix(token: &str, short: &str, long: &str) -> Option<(String, u32)> {
    // The token starts with the keyword, followed by optional digits.
    // Try long form first, then short form, picking the longest keyword match
    // that leaves trailing digits.
    for kw in &[long, short] {
        if token.len() >= kw.len() && token[..kw.len()] == **kw {
            let rest = &token[kw.len()..];
            if rest.is_empty() {
                // No suffix present -> default 1.
                return Some((kw.to_string(), 1));
            }
            if let Ok(n) = rest.parse::<u32>() {
                return Some((kw.to_string(), n));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Matching engine
// ---------------------------------------------------------------------------

/// Try to match `tokens` against `entry`, return suffixes + whether query
/// matched.  `is_query` comes from input, `entry.is_query` from pattern.
fn try_match(
    entry: &Entry,
    tokens: &[String],
    input_is_query: bool,
) -> Option<Vec<u32>> {
    if input_is_query != entry.is_query {
        return None;
    }

    // Build the set of acceptable segment sequences by expanding optional
    // groups.  Each optional group can be present or absent.
    // For simplicity (command tables are small), enumerate all 2^N combos.
    let n_opt = entry.optional_groups.len();
    let combos = 1u32 << n_opt;

    for combo in 0..combos {
        // Build the sequence of segments for this combo.
        let mut active_segments: Vec<&Segment> = Vec::new();
        for (idx, seg) in entry.segments.iter().enumerate() {
            // Check if this segment index falls inside a skipped optional group.
            let mut skipped = false;
            for (g, grp) in entry.optional_groups.iter().enumerate() {
                if idx >= grp.start && idx < grp.end && (combo >> g) & 1 == 0 {
                    skipped = true;
                    break;
                }
            }
            if !skipped {
                active_segments.push(seg);
            }
        }

        if let Some(suffixes) = match_segments(&active_segments, tokens) {
            return Some(suffixes);
        }
    }

    None
}

/// Match a flat list of active segments against input tokens.  Returns suffix
/// values on success.
fn match_segments(segments: &[&Segment], tokens: &[String]) -> Option<Vec<u32>> {
    let mut suffixes = Vec::new();
    let mut ti = 0; // token index
    let mut si = 0; // segment index

    while si < segments.len() {
        match &segments[si] {
            Segment::Keyword { short, long } => {
                if ti >= tokens.len() {
                    return None;
                }
                let token = &tokens[ti];

                // Check if next segment is NumericSuffix — if so, the suffix
                // digits may be glued onto this token.
                let next_is_suffix =
                    si + 1 < segments.len() && matches!(segments[si + 1], Segment::NumericSuffix);

                if next_is_suffix {
                    let (_, suf) = extract_suffix(token, short, long)?;
                    suffixes.push(suf);
                    si += 2; // consume keyword + suffix segment
                    ti += 1;
                } else {
                    if !keyword_matches(short, long, token) {
                        return None;
                    }
                    si += 1;
                    ti += 1;
                }
            }
            Segment::NumericSuffix => {
                // Standalone suffix (shouldn't normally happen since `#` follows
                // a keyword, but handle gracefully).
                if ti >= tokens.len() {
                    return None;
                }
                if let Ok(n) = tokens[ti].parse::<u32>() {
                    suffixes.push(n);
                    ti += 1;
                    si += 1;
                } else {
                    return None;
                }
            }
        }
    }

    // All segments consumed, all tokens consumed.
    if ti == tokens.len() {
        Some(suffixes)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Parameter parsing
// ---------------------------------------------------------------------------

/// Strip a trailing unit suffix (e.g. `"Hz"`, `"V"`, `"MHz"`) from a numeric
/// string, being careful not to eat the `e`/`E` of scientific notation.
fn strip_unit_suffix(raw: &str) -> &str {
    // Walk backwards past ASCII letters.
    let bytes = raw.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_alphabetic() {
        end -= 1;
    }
    if end == bytes.len() {
        return raw; // no trailing alpha
    }
    if end == 0 {
        return raw; // all alpha — not a valid number, let caller error
    }
    // Check if the "suffix" we'd strip is actually scientific notation:
    // the char at `end` is 'e'/'E' and preceded by a digit.
    let suffix = &raw[end..];
    if (suffix.starts_with('e') || suffix.starts_with('E'))
        && bytes[end - 1].is_ascii_digit()
        && suffix[1..].chars().all(|c| c.is_ascii_digit() || c == '+' || c == '-')
    {
        // This is scientific notation, not a unit suffix.
        return raw;
    }
    raw[..end].trim_end()
}

fn parse_param(kind: ParamKind, raw: &str) -> Result<Param, ParseError> {
    let raw = raw.trim();
    match kind {
        ParamKind::Num => {
            // Support hex (#H), octal (#Q), binary (#B) prefixes from SCPI.
            // Strip trailing unit suffix (e.g. "Hz", "V") — find the last
            // run of purely alphabetic chars that isn't part of scientific
            // notation (e/E followed by digits).
            let num_str = strip_unit_suffix(raw);
            if num_str.starts_with("#H") || num_str.starts_with("#h") {
                let val =
                    u64::from_str_radix(&num_str[2..], 16).map_err(|e| ParseError(e.to_string()))?;
                Ok(Param::Numeric(val as f64))
            } else if num_str.starts_with("#Q") || num_str.starts_with("#q") {
                let val =
                    u64::from_str_radix(&num_str[2..], 8).map_err(|e| ParseError(e.to_string()))?;
                Ok(Param::Numeric(val as f64))
            } else if num_str.starts_with("#B") || num_str.starts_with("#b") {
                let val =
                    u64::from_str_radix(&num_str[2..], 2).map_err(|e| ParseError(e.to_string()))?;
                Ok(Param::Numeric(val as f64))
            } else {
                let val: f64 = num_str
                    .parse()
                    .map_err(|e: std::num::ParseFloatError| ParseError(e.to_string()))?;
                Ok(Param::Numeric(val))
            }
        }
        ParamKind::Bool => {
            let upper = raw.to_ascii_uppercase();
            match upper.as_str() {
                "ON" | "1" => Ok(Param::Bool(true)),
                "OFF" | "0" => Ok(Param::Bool(false)),
                _ => Err(ParseError(format!("invalid boolean: {raw}"))),
            }
        }
        ParamKind::Str => {
            // Strip matching quotes if present.
            if (raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\''))
            {
                Ok(Param::String(raw[1..raw.len() - 1].to_string()))
            } else {
                Ok(Param::String(raw.to_string()))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CommandSet
// ---------------------------------------------------------------------------

impl CommandSet {
    /// Build a command set from a static table of `(pattern, handler)` pairs.
    pub fn from_table(table: &[(&str, Handler)]) -> Result<Self, ParseError> {
        let mut entries = Vec::with_capacity(table.len());
        for (pattern, handler) in table {
            let (segments, optional_groups, is_query, param) = compile(pattern)?;
            entries.push(Entry {
                segments,
                optional_groups,
                is_query,
                param,
                handler: *handler,
            });
        }
        Ok(CommandSet { entries })
    }

    /// Parse an input line (which may contain multiple `;`-separated commands)
    /// into a list of [`Command`]s.
    pub fn parse(&self, line: &str) -> Result<Vec<Command>, ParseError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(vec![]);
        }

        // Split on `;`, respecting quotes.
        let raw_cmds = split_commands(line);
        let mut result = Vec::new();

        for raw in &raw_cmds {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let cmd = self.parse_single(raw)?;
            result.push(cmd);
        }

        Ok(result)
    }

    fn parse_single(&self, input: &str) -> Result<Command, ParseError> {
        let (tokens, param_str) = tokenise_command(input);
        if tokens.is_empty() {
            return Err(ParseError("empty command".into()));
        }

        // Determine if this is a query (last token ends with `?`).
        let mut tokens = tokens;
        let mut is_query = false;
        if let Some(last) = tokens.last_mut() {
            if last.ends_with('?') {
                is_query = true;
                last.truncate(last.len() - 1);
                if last.is_empty() {
                    // Standalone `?` — shouldn't happen in practice.
                    tokens.pop();
                }
            }
        }

        // Try each entry.
        for (idx, entry) in self.entries.iter().enumerate() {
            if let Some(suffixes) = try_match(entry, &tokens, is_query) {
                // Parse parameter if expected.
                let params = if let Some(kind) = entry.param {
                    let raw = param_str.as_deref().unwrap_or("");
                    if raw.is_empty() {
                        return Err(ParseError(format!(
                            "command expects a parameter but none given"
                        )));
                    }
                    vec![parse_param(kind, raw)?]
                } else {
                    vec![]
                };
                return Ok(Command {
                    index: idx,
                    params,
                    suffixes,
                });
            }
        }

        Err(ParseError(format!("unrecognised command: {input}")))
    }

    /// Dispatch a parsed command to its handler.
    pub fn dispatch(&self, cmd: &Command) {
        (self.entries[cmd.index].handler)(cmd);
    }
}

// ---------------------------------------------------------------------------
// Line splitting
// ---------------------------------------------------------------------------

/// Split a line on `;` while respecting quoted strings.
fn split_commands(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = ' ';

    for ch in line.chars() {
        if in_quotes {
            current.push(ch);
            if ch == quote_char {
                in_quotes = false;
            }
        } else if ch == '\'' || ch == '"' {
            in_quotes = true;
            quote_char = ch;
            current.push(ch);
        } else if ch == ';' {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy(_: &Command) {}

    #[test]
    fn common_command() {
        let table: &[(&str, Handler)] = &[("*IDN?", dummy), ("*RST", dummy)];
        let set = CommandSet::from_table(table).unwrap();

        let cmds = set.parse("*IDN?").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].index, 0);
        assert!(cmds[0].params.is_empty());

        let cmds = set.parse("*RST").unwrap();
        assert_eq!(cmds[0].index, 1);
    }

    #[test]
    fn common_with_param() {
        let table: &[(&str, Handler)] = &[("*ESE num", dummy)];
        let set = CommandSet::from_table(table).unwrap();

        let cmds = set.parse("*ESE 42").unwrap();
        assert_eq!(cmds[0].index, 0);
        assert!(matches!(cmds[0].params[0], Param::Numeric(v) if v == 42.0));
    }

    #[test]
    fn hierarchical_short_long() {
        let table: &[(&str, Handler)] = &[("SYSTem:VERSion?", dummy)];
        let set = CommandSet::from_table(table).unwrap();

        // Short form.
        assert!(set.parse("SYST:VERS?").is_ok());
        // Long form.
        assert!(set.parse("system:version?").is_ok());
        // Mixed case.
        assert!(set.parse("System:Version?").is_ok());
    }

    #[test]
    fn optional_node() {
        let table: &[(&str, Handler)] = &[("SYSTem:ERRor[:NEXT]?", dummy)];
        let set = CommandSet::from_table(table).unwrap();

        // With optional node.
        assert!(set.parse("SYST:ERR:NEXT?").is_ok());
        // Without optional node.
        assert!(set.parse("SYST:ERR?").is_ok());
    }

    #[test]
    fn numeric_suffix() {
        let table: &[(&str, Handler)] = &[("SOURce#:FREQuency num", dummy)];
        let set = CommandSet::from_table(table).unwrap();

        let cmds = set.parse("SOUR2:FREQ 1e6").unwrap();
        assert_eq!(cmds[0].suffixes[0], 2);
        assert!(matches!(cmds[0].params[0], Param::Numeric(v) if v == 1e6));

        // Default suffix = 1.
        let cmds = set.parse("SOURCE:FREQ 500").unwrap();
        assert_eq!(cmds[0].suffixes[0], 1);
    }

    #[test]
    fn bool_param() {
        let table: &[(&str, Handler)] = &[("OUTPut#:STATe bool", dummy)];
        let set = CommandSet::from_table(table).unwrap();

        let cmds = set.parse("OUTP1:STAT ON").unwrap();
        assert!(matches!(cmds[0].params[0], Param::Bool(true)));

        let cmds = set.parse("OUTPUT2:STATE 0").unwrap();
        assert!(matches!(cmds[0].params[0], Param::Bool(false)));
        assert_eq!(cmds[0].suffixes[0], 2);
    }

    #[test]
    fn string_param() {
        let table: &[(&str, Handler)] = &[("DISPlay:TEXT str", dummy)];
        let set = CommandSet::from_table(table).unwrap();

        let cmds = set.parse("DISP:TEXT \"hello world\"").unwrap();
        assert!(matches!(&cmds[0].params[0], Param::String(s) if s == "hello world"));
    }

    #[test]
    fn multi_command_line() {
        let table: &[(&str, Handler)] = &[("*RST", dummy), ("*IDN?", dummy)];
        let set = CommandSet::from_table(table).unwrap();

        let cmds = set.parse("*RST;*IDN?").unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].index, 0);
        assert_eq!(cmds[1].index, 1);
    }

    #[test]
    fn optional_voltage_level() {
        let table: &[(&str, Handler)] = &[
            ("SOURce#:VOLTage[:LEVel] num", dummy),
            ("SOURce#:VOLTage[:LEVel]?", dummy),
        ];
        let set = CommandSet::from_table(table).unwrap();

        // With optional :LEVel
        assert!(set.parse("SOUR1:VOLT:LEV 3.3").is_ok());
        // Without
        assert!(set.parse("SOUR1:VOLT 3.3").is_ok());
        // Query with
        assert!(set.parse("SOUR1:VOLT:LEVEL?").is_ok());
        // Query without
        assert!(set.parse("SOUR1:VOLT?").is_ok());
    }
}
