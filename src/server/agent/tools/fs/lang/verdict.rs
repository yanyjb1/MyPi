// What a language's formatter and checker answer: the neutral shapes every tool reads.

// What a formatter said about a text it could not parse.
#[derive(Debug, Clone)]
pub(crate) struct Unparsed {
    // The line it pointed at, 1-based, in the text it was given.
    pub line: Option<usize>,

    // What it said, in one line.
    pub message: String,
}

// One run of text a call just wrote, read in the answer's own coordinates: its first and last line, both 1-based; `first > last` when it left no line, `None` when the formatter left no trace of it.
pub(crate) type Place = Option<(usize, usize)>;

// One text on its way to a formatter: the whole text, and where the runs a call just wrote sit inside it.
pub(crate) struct Draft<'a> {
    // The text handed over, every written run already in place.
    pub(crate) text: &'a str,
    // One entry per written run, 1-based inclusive, in the order the caller handed them over; `first > last` when the run left no line of its own.
    pub(crate) runs: &'a [(usize, usize)],
}

// One text's turn through a formatter.
#[derive(Debug, Clone)]
pub(crate) enum Outcome {
    // The text as the formatter left it, and where each run of the draft sits in it, in the draft's own order.
    Formatted { text: String, runs: Vec<Place> },

    // The formatter ran and said the text does not parse.
    Unparsed(Unparsed),

    // The formatter did not finish or could not run: nothing is known about the text.
    Undecided(String),

    // The turn was stopped: the formatter was killed, and its answer means nothing.
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
