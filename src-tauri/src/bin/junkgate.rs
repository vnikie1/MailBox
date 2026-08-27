//! Phase 8 exit gate for the junk filter: **>90% on a labelled corpus**. docs/06 Phase 8.
//!
//! ## Why not the user's own mailbox
//!
//! The first version of this gate scored the live database and reported **97.3% accuracy** —
//! while catching **0.3% of the junk**. Both numbers were correct and the gate was worthless:
//! the corpus was 97% ham, so a classifier that returns "clean" for everything scores 97.2%.
//! An accuracy figure on an imbalanced corpus measures the imbalance, not the classifier.
//!
//! It was also the wrong corpus. Almost all of that mail was seeded test data whose Junk folder
//! holds randomly assigned generated text — nothing distinguishes it from the Inbox, so nothing
//! could classify it. The one real account had **twelve** messages in Spam.
//!
//! So the gate uses the **SpamAssassin public corpus**: human-labelled, published for exactly
//! this purpose, and not written by me — a corpus I wrote myself would measure only whether I
//! can write convincing spam. Nothing from the user's mailbox is read, and nothing leaves this
//! machine.
//!
//! ```text
//! curl -O https://spamassassin.apache.org/old/publiccorpus/20030228_easy_ham.tar.bz2
//! curl -O https://spamassassin.apache.org/old/publiccorpus/20030228_hard_ham.tar.bz2
//! curl -O https://spamassassin.apache.org/old/publiccorpus/20030228_spam.tar.bz2
//! curl -O https://spamassassin.apache.org/old/publiccorpus/20050311_spam_2.tar.bz2
//! for f in *.tar.bz2; do tar -xf "$f"; done
//! cargo run --bin junkgate -- <that directory>
//! ```
//!
//! ## What is measured
//!
//! Trained and tested on **disjoint halves**. Scoring the messages it trained on would report a
//! number that means nothing — a classifier can memorise its training set perfectly and be
//! useless on everything else.
//!
//! The headline is **balanced accuracy** — the mean of the junk-caught rate and the
//! clean-kept rate — because it is the metric a do-nothing classifier cannot pass. Answering
//! "clean" for everything scores 50%, whatever the corpus mix.
//!
//! Three floors, all of which must hold:
//!
//! | measure | floor | why |
//! |---|---|---|
//! | balanced accuracy | 90% | the gate docs/06 asks for, stated so it cannot be gamed |
//! | junk caught | 80% | a filter that catches nothing is not a filter |
//! | real mail misfiled | at most 0.5% | a false positive hides a message the user needed |
//!
//! The last one is the one that matters. A false negative leaves one more piece of spam in the
//! Inbox, which is an annoyance; a false positive hides a real message, which is a missed
//! flight, invoice or job offer. Those costs are nowhere near equal and the gate should not
//! pretend they are.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use halcyon_lib::rules::junk::{self, Classifier, Verdict};
use rusqlite::Connection;

struct Example {
    from: String,
    subject: String,
    body: String,
    is_junk: bool,
}

/// Which corpus directories hold which label. The SpamAssassin layout, and it is worth keeping
/// `hard_ham` in: it is legitimate mail that *looks* like spam — marketing a user opted into,
/// HTML newsletters, offers from real companies. Dropping it would remove exactly the cases the
/// false-positive floor exists to protect.
const CORPUS: &[(&str, bool)] = &[
    ("easy_ham", false),
    ("hard_ham", false),
    ("spam", true),
    ("spam_2", true),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root: PathBuf = match std::env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => {
            eprintln!("usage: cargo run --bin junkgate -- <corpus directory>");
            eprintln!("see the module comment for where to get one");
            std::process::exit(2);
        }
    };

    let mut examples = Vec::new();

    println!("corpus at {}", root.display());

    for (folder, is_junk) in CORPUS {
        let directory = root.join(folder);
        if !directory.is_dir() {
            println!("  {folder:<12} (absent)");
            continue;
        }

        let mut count = 0;

        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }

            // Each corpus directory carries a `cmds` file that is not mail.
            if path.file_name().is_some_and(|name| name == "cmds") {
                continue;
            }

            if let Some(example) = parse(&path, *is_junk) {
                examples.push(example);
                count += 1;
            }
        }

        println!("  {folder:<12} {count}");
    }

    let junk_total = examples.iter().filter(|e| e.is_junk).count();
    let ham_total = examples.len() - junk_total;

    println!("\n{ham_total} ham, {junk_total} junk");

    if junk_total < 100 || ham_total < 100 {
        eprintln!("\nnot enough on one side to measure anything; neither a pass nor a fail");
        std::process::exit(3);
    }

    // Disjoint halves, alternating **within each label**, so both halves keep the corpus mix.
    // Splitting the concatenated list down the middle would train on ham and test on spam.
    let mut seen_ham = 0usize;
    let mut seen_junk = 0usize;
    let mut train_set = Vec::new();
    let mut test_set = Vec::new();

    for example in &examples {
        let index = if example.is_junk {
            seen_junk += 1;
            seen_junk
        } else {
            seen_ham += 1;
            seen_ham
        };

        if index % 2 == 0 {
            train_set.push(example);
        } else {
            test_set.push(example);
        }
    }

    println!(
        "train {} / test {} (disjoint, stratified)",
        train_set.len(),
        test_set.len()
    );

    // In memory: the corpus tables are the only thing written, and a gate has no business
    // touching the user's database at all.
    let mut conn = Connection::open_in_memory()?;
    halcyon_lib::db::migrate::run(&mut conn)?;

    let tx = conn.transaction()?;
    for example in &train_set {
        junk::train(
            &tx,
            &example.from,
            &example.subject,
            &example.body,
            example.is_junk,
        )?;
    }
    tx.commit()?;

    let classifier = Classifier::load(&conn)?;
    if !classifier.ready() {
        eprintln!(
            "classifier declined to answer below the {} minimum",
            junk::MIN_CORPUS
        );
        std::process::exit(3);
    }

    // Two policy dials, and neither should be set by taste. The sweep shows what each pair
    // costs on real mail before the verdict is read.
    println!(
        "
  tokens  minocc  threshold   caught   misfiled   balanced"
    );
    let mut tuning = classifier;
    for (significant, minocc) in [
        (15_usize, 2_i64),
        (27, 2),
        (27, 5),
        (27, 10),
        (50, 5),
        (50, 10),
    ] {
        tuning.significant = significant;
        tuning.min_occurrences = minocc;

        for step in [0.90_f64, 0.95, 0.99] {
            let mut tj = 0i64;
            let mut fh = 0i64;
            let mut fj = 0i64;
            let mut th = 0i64;

            for example in &test_set {
                let above = tuning
                    .score(&example.from, &example.subject, &example.body)
                    .probability()
                    .is_some_and(|p| p >= step);

                match (example.is_junk, above) {
                    (true, true) => tj += 1,
                    (true, false) => fh += 1,
                    (false, true) => fj += 1,
                    (false, false) => th += 1,
                }
            }

            let c = tj as f64 / (tj + fh).max(1) as f64;
            let k = th as f64 / (th + fj).max(1) as f64;
            println!(
                "  {:>6}  {:>6}  {:>8.2}   {:>5.1}%   {:>7.2}%   {:>7.2}%",
                significant,
                minocc,
                step,
                c * 100.0,
                (1.0 - k) * 100.0,
                (c + k) / 2.0 * 100.0
            );
        }
    }
    // Reloaded rather than reusing the swept one: `tuning` is left holding the *last* row of
    // the table above, and scoring the verdict with it would report a setting nobody ships.
    // The first version of this did exactly that and reported a failure that was not real.
    drop(tuning);
    let classifier = Classifier::load(&conn)?;

    // The threshold is a policy choice, not a constant to guess at, so the gate shows what
    // each one costs before the verdict. Every row is the same trained classifier; only the
    // line between junk and clean moves.
    println!(
        "
  threshold   caught   misfiled   balanced"
    );
    for step in [0.50_f64, 0.70, 0.80, 0.90, 0.95, 0.98, 0.99, 0.995, 0.999] {
        let mut tj = 0i64;
        let mut fh = 0i64;
        let mut fj = 0i64;
        let mut th = 0i64;

        for example in &test_set {
            let above = classifier
                .score(&example.from, &example.subject, &example.body)
                .probability()
                .is_some_and(|p| p >= step);

            match (example.is_junk, above) {
                (true, true) => tj += 1,
                (true, false) => fh += 1,
                (false, true) => fj += 1,
                (false, false) => th += 1,
            }
        }

        let c = tj as f64 / (tj + fh).max(1) as f64;
        let k = th as f64 / (th + fj).max(1) as f64;
        println!(
            "  {:>8.3}   {:>5.1}%   {:>7.2}%   {:>7.2}%",
            step,
            c * 100.0,
            (1.0 - k) * 100.0,
            (c + k) / 2.0 * 100.0
        );
    }

    let mut matrix: HashMap<(bool, bool), i64> = HashMap::new();
    let mut undecided = 0;

    for example in &test_set {
        let verdict = classifier.score(&example.from, &example.subject, &example.body);

        if matches!(verdict, Verdict::Undecided) {
            undecided += 1;
        }

        *matrix
            .entry((example.is_junk, verdict.is_junk()))
            .or_insert(0) += 1;
    }

    let at = |actual: bool, predicted: bool| *matrix.get(&(actual, predicted)).unwrap_or(&0);

    let true_junk = at(true, true);
    let false_ham = at(true, false); // junk that got through
    let false_junk = at(false, true); // real mail called junk — the expensive one
    let true_ham = at(false, false);

    let caught = true_junk as f64 / (true_junk + false_ham).max(1) as f64;
    let kept = true_ham as f64 / (true_ham + false_junk).max(1) as f64;
    let balanced = (caught + kept) / 2.0;
    let misfiled = 1.0 - kept;

    let total = (true_junk + false_ham + false_junk + true_ham).max(1) as f64;
    let plain = (true_junk + true_ham) as f64 / total;

    println!("\n                 called junk   called clean");
    println!("  is junk        {true_junk:>11}   {false_ham:>12}");
    println!("  is clean       {false_junk:>11}   {true_ham:>12}");
    println!("\n  undecided (treated as clean): {undecided}");
    println!(
        "  plain accuracy       {:>6.2}%   (for reference only)",
        plain * 100.0
    );
    println!(
        "  junk caught          {:>6.2}%   floor 80%",
        caught * 100.0
    );
    println!(
        "  real mail misfiled   {:>6.2}%   ceiling 0.5%",
        misfiled * 100.0
    );
    println!(
        "  BALANCED ACCURACY    {:>6.2}%   floor 90%",
        balanced * 100.0
    );

    // Printed every run so nobody has to work out whether the number is any good. This is what
    // a classifier that simply answers "clean" every time would score.
    println!(
        "\n  a do-nothing filter scores 50% balanced, {:.2}% plain on this corpus",
        (true_ham + false_junk) as f64 / total * 100.0
    );

    let pass = balanced >= 0.90 && caught >= 0.80 && misfiled <= 0.005;

    if pass {
        println!("\nGATE PASSED");
        Ok(())
    } else {
        println!("\nGATE FAILED");
        std::process::exit(1);
    }
}

/// Pulls `From`, `Subject` and the body out of an RFC 822 file.
///
/// Deliberately crude — unfolding and a blank-line split, no MIME walk. The classifier is fed
/// what the message row would hold, and a corpus parsed more carefully than production data
/// would flatter the result.
fn parse(path: &Path, is_junk: bool) -> Option<Example> {
    let bytes = std::fs::read(path).ok()?;
    // The corpus is twenty years old and contains every encoding there has ever been. Lossy is
    // right here: a message that fails to decode cleanly is still a message to classify.
    let text = String::from_utf8_lossy(&bytes);

    let mut from = String::new();
    let mut subject = String::new();
    let mut body = String::new();
    let mut in_headers = true;
    let mut continuing: Option<Header> = None;

    for line in text.lines() {
        if in_headers {
            if line.trim().is_empty() {
                in_headers = false;
                continue;
            }

            if line.starts_with(' ') || line.starts_with('\t') {
                let target = match continuing {
                    Some(Header::From) => &mut from,
                    Some(Header::Subject) => &mut subject,
                    None => continue,
                };

                target.push(' ');
                target.push_str(line.trim());
                continue;
            }

            let lower = line.to_lowercase();

            if let Some(value) = lower.strip_prefix("from:") {
                from = line[line.len() - value.len()..].trim().to_string();
                continuing = Some(Header::From);
            } else if let Some(value) = lower.strip_prefix("subject:") {
                subject = line[line.len() - value.len()..].trim().to_string();
                continuing = Some(Header::Subject);
            } else {
                continuing = None;
            }

            continue;
        }

        body.push_str(line);
        body.push('\n');

        if body.len() > 8_000 {
            break;
        }
    }

    Some(Example {
        from,
        subject,
        body,
        is_junk,
    })
}

#[derive(Clone, Copy)]
enum Header {
    From,
    Subject,
}
