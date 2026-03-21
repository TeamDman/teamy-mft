use nucleo::Config;
use nucleo::Nucleo;
use nucleo::pattern::CaseMatching;
use nucleo::pattern::Normalization;
use std::sync::Arc;

const FILES: &[&str] = &["a.txt", "b.json", "c.webm", "d.mp4"];

fn run_query(query: &str) -> Vec<String> {
    let mut nucleo = Nucleo::<String>::new(Config::DEFAULT, Arc::new(|| {}), Some(1), 1);
    let injector = nucleo.injector();

    for path in FILES {
        injector.push((*path).to_owned(), |value, columns| {
            columns[0] = value.clone().into();
        });
    }

    nucleo
        .pattern
        .reparse(0, query, CaseMatching::Smart, Normalization::Smart, false);

    loop {
        let status = nucleo.tick(10);
        if !status.running {
            break;
        }
    }

    let mut matches: Vec<String> = nucleo
        .snapshot()
        .matched_items(..)
        .map(|item| item.data.clone())
        .collect();
    matches.sort();
    matches
}

#[test]
fn postfix_query_matches_webm_extension() {
    assert_eq!(run_query(".webm$"), vec!["c.webm"]);
}

#[test]
fn postfix_query_matches_mp4_extension() {
    assert_eq!(run_query(".mp4$"), vec!["d.mp4"]);
}

#[test]
fn fuzzy_query_can_match_both_media_files() {
    assert_eq!(run_query("m"), vec!["c.webm", "d.mp4"]);
}

#[test]
fn multi_word_query_acts_like_an_and_filter() {
    assert_eq!(run_query("d mp4"), vec!["d.mp4"]);
}

#[test]
fn pipe_or_query_is_not_supported() {
    assert!(run_query(".webm$ | .mp4$").is_empty());
}

#[test]
fn whitespace_separated_postfix_terms_are_anded() {
    assert!(run_query(".webm$ .mp4$").is_empty());
}
