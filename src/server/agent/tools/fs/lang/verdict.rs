// What a language's parser, checker and test run answer: the neutral shapes every tool reads.

// What a parser said about a text it could not parse.
#[derive(Debug, Clone)]
pub(crate) struct Unparsed {
    // The line it pointed at, 1-based, in the text it was given.
    pub line: Option<usize>,

    // The column it pointed at, 1-based, when the parser gives one.
    pub column: Option<usize>,

    // What it said, in one line.
    pub message: String,
}

// One text's turn through a language's parser. The parser judges the text and never rewrites it: a text it accepts is written exactly as the caller handed it over.
#[derive(Debug, Clone)]
pub(crate) enum Outcome {
    // The text parses.
    Parsed,

    // The parser ran and said the text does not parse.
    Unparsed(Unparsed),

    // The turn was stopped: the parser was killed, and its answer means nothing.
    Stopped,
}

// What one unit's tests had to say about the tests a batch just wrote.
#[derive(Debug)]
pub(crate) enum Verdict {
    // Nothing was verified: no test was found, the tests do not compile, or the turn was stopped.
    Unchecked(String),

    // Every test that ran passed; `note` says what the run could not answer for.
    Passed { ran: usize, note: Option<String> },

    // At least one test failed, each with the file and line its panic named.
    Failed { faults: Vec<Fault>, summary: String },
}

// What a checker had to say about one unit after a batch wrote into it.
#[derive(Debug)]
pub(crate) enum Check {
    // Nothing was verified: the checker did not run, or the turn was stopped.
    Unchecked(String),

    // No errors and no warnings.
    Clean,

    // Every error first, then every warning, in the checker's own order.
    Faults {
        faults: Vec<Fault>,
        warns: Vec<Fault>,
    },
}

// One diagnostic, cut to what a model acts on.
#[derive(Debug, Clone)]
pub(crate) struct Fault {
    // The file, named the way its own unit names it.
    pub file: String,

    // The line it points at, 1-based.
    pub line: usize,

    // What went wrong.
    pub message: String,

    // What the checker suggests, its own words.
    pub helps: Vec<String>,
}
