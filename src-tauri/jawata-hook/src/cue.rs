//! Extract cues from a user's prompt — the one piece of this binary with real
//! algorithmic content, and the place the hook outage lived.
//!
//! Ported from the generated `UserPromptSubmit` shell script, semantics
//! preserved deliberately: this is a rewrite of the CARRIER, not of the
//! policy. Two things change, and both are the point of the rewrite:
//!
//! * **Skipping is a value, not a `return`.** The script's every dead end was
//!   a bare `exit 0`, indistinguishable from "asked and got nothing". Here a
//!   skip is a [`SkipReason`] the caller can record, which is what makes
//!   Stage 8's silence log possible.
//! * **No shelling out.** The script normalised with `tr`/`sed` and built
//!   n-grams with `awk`, so its behaviour depended on which awk the user had.
//!
//! Identifier scanning is hand-rolled rather than pulling in `regex`: the
//! three shapes are simple, and a hook that fires on every prompt should not
//! pay a regex engine's compile cost to find them.

/// Why a prompt produced no cues. Never a bare absence — an absence with a
/// reason is the difference between "nothing to say" and "we never asked".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// No `prompt` field, or it was empty.
    EmptyPrompt,
    /// A slash command (`/foo`) — the user is driving the client, not asking
    /// a question the store could answer.
    SlashCommand,
    /// Fewer than two content-bearing tokens survived the stopword filter.
    /// One token is not a topic; recalling on it returns noise.
    TooFewContentTokens { found: usize },
}

/// The cues a prompt yields, in the order they should be tried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cues {
    /// Qualified/member identifiers (`Type#member`, `pkg.Type`, `Outer.Inner`),
    /// case-sensitive, from the ORIGINAL prompt. Tried FIRST and independently
    /// of the two-token gate: a bare `Foo#bar` prompt must still recall.
    pub symbols: Vec<String>,
    /// Symptom cues: the best trigram, then two bigrams. Trigrams buy
    /// precision; bigrams are the workhorse, and they get two of the three
    /// attempts so a long prompt can never starve them.
    pub symptoms: Vec<String>,
    /// Content tokens that survived filtering — reported so the silence log
    /// can say how close a skipped prompt came.
    pub content_tokens: usize,
}

/// The stopword list, carried over verbatim. Deliberately small and closed:
/// it removes grammar, never domain words.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "for", "with", "this", "that", "these", "those", "is", "are",
    "was", "were", "be", "been", "to", "of", "in", "on", "at", "it", "its", "we", "i", "you", "he",
    "she", "they", "do", "does", "did", "not", "no", "yes", "our", "my", "your", "his", "her",
    "their", "what", "which", "how", "why", "when", "where", "who", "make", "makes", "made",
    "making", "please", "now", "then", "so", "but", "if", "else", "can", "could", "should",
    "would", "will", "shall", "may", "might", "must", "have", "has", "had", "get", "got", "just",
    "also", "about", "into", "from", "out", "up", "down", "over", "again", "more", "less", "very",
    "all", "any", "some", "one", "two", "new", "use", "used", "using",
];

/// Cap on content tokens considered. A prompt is a question, not a corpus;
/// beyond this the n-grams are about the prompt's tail, not its subject.
const MAX_TOKENS: usize = 40;

/// Cap on symbol cues tried. Two precise attempts, then symptoms.
const MAX_SYMBOLS: usize = 2;

/// Extract cues, or say why there are none.
pub fn extract(prompt: &str) -> Result<Cues, SkipReason> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(SkipReason::EmptyPrompt);
    }
    if prompt.starts_with('/') {
        return Err(SkipReason::SlashCommand);
    }

    let symbols = symbol_cues(prompt);
    let tokens = content_tokens(prompt);

    // The symbol path is INDEPENDENT of the two-token gate: `Foo#bar` alone is
    // a precise question even though it is one token.
    if tokens.len() < 2 {
        if symbols.is_empty() {
            return Err(SkipReason::TooFewContentTokens { found: tokens.len() });
        }
        return Ok(Cues { symbols, symptoms: Vec::new(), content_tokens: tokens.len() });
    }

    let tri = ngrams(&tokens, 3);
    let bi = ngrams(&tokens, 2);
    let mut symptoms = Vec::new();
    symptoms.extend(tri.into_iter().take(1));
    symptoms.extend(bi.into_iter().take(2));

    Ok(Cues { symbols, symptoms, content_tokens: tokens.len() })
}

/// Content tokens: lowercased, punctuation split, stopwords and short words
/// dropped. Digits, hyphens and underscores SURVIVE — they are the rarity
/// markers the n-gram tiering reads.
fn content_tokens(prompt: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in prompt.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            current.push(c);
        } else {
            push_token(&mut out, &mut current);
            if out.len() >= MAX_TOKENS {
                return out;
            }
        }
    }
    push_token(&mut out, &mut current);
    out.truncate(MAX_TOKENS);
    out
}

fn push_token(out: &mut Vec<String>, current: &mut String) {
    if current.len() >= 3 && !STOPWORDS.contains(&current.as_str()) {
        out.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

/// N-grams in TIER order: within a tier, rarity-marked cues (containing a
/// digit, hyphen or underscore) come before plain ones, then order of
/// appearance. Duplicates are dropped on first sight.
///
/// DECLARED deviation from "rarer tokens first", carried over from the script:
/// true corpus rarity would need a frequency table the hook does not have, so
/// the marker heuristic is the deterministic proxy.
fn ngrams(tokens: &[String], len: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    if tokens.len() < len {
        return out;
    }
    for want_marked in [true, false] {
        for i in 0..=(tokens.len() - len) {
            let cue = tokens[i..i + len].join(" ");
            let marked = cue.chars().any(|c| c.is_ascii_digit() || c == '_' || c == '-');
            if marked == want_marked && seen.insert(cue.clone()) {
                out.push(cue);
            }
        }
    }
    out
}

/// Qualified identifiers, scanned case-sensitively from the original prompt.
///
/// Three shapes, in the script's own order of precision:
/// `Type#member` / `pkg.Type#member` · `pkg.Type` (lowercase segments then an
/// uppercase one) · `Outer.Inner` (all uppercase-initial segments).
fn symbol_cues(prompt: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in prompt.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '#')) {
        let candidate = raw.trim_matches(|c| c == '.' || c == '#');
        if candidate.is_empty() || !is_symbol_shape(candidate) {
            continue;
        }
        if !out.iter().any(|s| s == candidate) {
            out.push(candidate.to_string());
            if out.len() >= MAX_SYMBOLS {
                break;
            }
        }
    }
    out
}

fn is_symbol_shape(s: &str) -> bool {
    // Member form: <dotted>#<member>
    if let Some((owner, member)) = s.split_once('#') {
        return !member.is_empty()
            && member.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !owner.is_empty()
            && owner.split('.').all(ident)
            && owner.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false);
    }
    if s.contains('#') {
        return false;
    }
    // Dotted forms need at least one dot and at least one uppercase-initial
    // segment; a bare lowercase word or a sentence's "e.g" must not qualify.
    let segments: Vec<&str> = s.split('.').collect();
    if segments.len() < 2 || !segments.iter().all(|seg| ident(seg)) {
        return false;
    }
    let last = segments[segments.len() - 1];
    let upper_first = |seg: &str| seg.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false);
    // pkg.Type — lowercase package segments, uppercase-initial type.
    let pkg_type = upper_first(last)
        && segments[..segments.len() - 1]
            .iter()
            .all(|seg| seg.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false));
    // Outer.Inner — every segment uppercase-initial.
    let nested = segments.iter().all(|seg| upper_first(seg));
    pkg_type || nested
}

fn ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slash_command_is_skipped_with_its_reason() {
        assert_eq!(Err(SkipReason::SlashCommand), extract("/memorize something"));
    }

    #[test]
    fn an_empty_prompt_is_skipped_with_its_reason() {
        assert_eq!(Err(SkipReason::EmptyPrompt), extract("   "));
    }

    #[test]
    fn one_content_token_is_not_a_topic() {
        // "the" and "is" are stopwords, "ok" is under three characters.
        assert_eq!(
            Err(SkipReason::TooFewContentTokens { found: 1 }),
            extract("is the deployment ok")
        );
    }

    #[test]
    fn a_bare_symbol_recalls_even_though_it_is_one_token() {
        // THE reason the symbol path bypasses the two-token gate.
        let cues = extract("ProjectImporter#isTestSourceRoot").unwrap();
        assert_eq!(vec!["ProjectImporter#isTestSourceRoot"], cues.symbols);
    }

    #[test]
    fn symbol_shapes_are_recognised_and_prose_is_not() {
        let cues = extract("does org.jawata.core.SourceRootClassifier handle it").unwrap();
        assert_eq!(vec!["org.jawata.core.SourceRootClassifier"], cues.symbols);

        // A sentence with a period must not become a "symbol".
        let prose = extract("the importer failed. the classifier recovered").unwrap();
        assert!(prose.symbols.is_empty(), "prose yielded symbols: {:?}", prose.symbols);
    }

    #[test]
    fn marked_ngrams_outrank_plain_ones_within_a_tier() {
        // "sprint-28" carries a hyphen, so its bigram is rarity-marked and
        // must be offered before the plain bigram that appears earlier.
        let cues = extract("importer classifier sprint-28 regression").unwrap();
        let first = &cues.symptoms[cues.symptoms.len() - 2];
        assert!(
            first.contains("sprint-28"),
            "marked cue should lead its tier, got {:?}",
            cues.symptoms
        );
    }

    #[test]
    fn one_trigram_then_two_bigrams() {
        let cues = extract("importer classifier regression scope detector").unwrap();
        assert_eq!(3, cues.symptoms.len(), "{:?}", cues.symptoms);
        assert_eq!(3, cues.symptoms[0].split(' ').count(), "first cue is the trigram");
        assert_eq!(2, cues.symptoms[1].split(' ').count());
        assert_eq!(2, cues.symptoms[2].split(' ').count());
    }

    #[test]
    fn stopwords_and_short_words_never_become_cues() {
        let cues = extract("why is the classifier not using the importer").unwrap();
        for cue in &cues.symptoms {
            for word in cue.split(' ') {
                assert!(!STOPWORDS.contains(&word), "stopword {word} in cue {cue}");
                assert!(word.len() >= 3, "short word {word} in cue {cue}");
            }
        }
    }

    // ---- properties -------------------------------------------------------
    //
    // The outage was a shape change producing an EMPTY result that read as
    // "nothing to say". These assert the shape of the answer over a wide input
    // space rather than on one example.

    fn corpus() -> Vec<String> {
        let mut out = Vec::new();
        let words = [
            "importer", "classifier", "sprint-28", "scope_main", "regression", "hook", "the",
            "is", "a", "detector", "baseline", "gate", "Foo#bar", "org.example.Thing", "/slash",
            "réfèrence", "12", "x", "", "...", "a.b.c",
        ];
        for i in 0..words.len() {
            for j in 0..words.len() {
                out.push(format!("{} {}", words[i], words[j]));
                out.push(format!("{} {} {}", words[i], words[j], words[(i + j) % words.len()]));
            }
        }
        out
    }

    #[test]
    fn property_never_panics_and_always_explains_itself() {
        for prompt in corpus() {
            match extract(&prompt) {
                Ok(cues) => assert!(
                    !cues.symbols.is_empty() || !cues.symptoms.is_empty(),
                    "a successful extraction with NO cues is the outage's exact shape — \
                     an empty answer indistinguishable from 'nothing to say': {prompt:?}"
                ),
                Err(_) => {}   // a reason is a reason; the type guarantees one exists
            }
        }
    }

    #[test]
    fn property_cues_are_bounded_and_nonempty() {
        for prompt in corpus() {
            if let Ok(cues) = extract(&prompt) {
                assert!(cues.symbols.len() <= MAX_SYMBOLS, "{prompt:?}");
                assert!(cues.symptoms.len() <= 3, "{prompt:?}");
                for cue in cues.symbols.iter().chain(cues.symptoms.iter()) {
                    assert!(!cue.trim().is_empty(), "blank cue from {prompt:?}");
                    assert!(!cue.contains('"'), "cue would break the JSON request: {cue:?}");
                    assert!(!cue.contains('\n'), "cue spans lines: {cue:?}");
                }
            }
        }
    }

    #[test]
    fn property_extraction_is_deterministic() {
        for prompt in corpus() {
            assert_eq!(extract(&prompt), extract(&prompt), "{prompt:?}");
        }
    }
}
