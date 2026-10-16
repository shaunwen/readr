use crate::db::{Book, Source};

pub fn matches(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    let mut words = query.split_whitespace().peekable();
    words.peek().is_some() && words.all(|word| text.contains(&word.to_lowercase()))
}

pub fn find_books<'a>(books: &'a [Book], query: &str) -> Vec<&'a Book> {
    let exact: Vec<&Book> = books
        .iter()
        .filter(|book| book.title.eq_ignore_ascii_case(query.trim()))
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    books
        .iter()
        .filter(|book| {
            let text = format!("{} {}", book.title, book.author.as_deref().unwrap_or(""));
            matches(query, &text)
        })
        .collect()
}

pub fn find_sources<'a>(sources: &'a [Source], query: &str) -> Vec<&'a Source> {
    let exact: Vec<&Source> = sources
        .iter()
        .filter(|source| {
            source.display_title().eq_ignore_ascii_case(query.trim()) || source.locator == query
        })
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    sources
        .iter()
        .filter(|source| {
            let text = format!(
                "{} {} {}",
                source.display_title(),
                source.author.as_deref().unwrap_or(""),
                source.locator
            );
            matches(query, &text)
        })
        .collect()
}

pub fn same_title(a: &str, b: &str) -> bool {
    normalize(a) == normalize(b)
}

fn normalize(title: &str) -> String {
    title
        .split([':', '('])
        .next()
        .unwrap_or(title)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;
    use crate::model::{BookStatus, Reader};

    fn book(id: i64, title: &str) -> Book {
        Book {
            id,
            title: title.into(),
            author: Some("Henry David Thoreau".into()),
            goal: None,
            status: BookStatus::Active,
            priority: 0,
            added_at: Timestamp::UNIX_EPOCH,
            finished_at: None,
        }
    }

    fn source(id: i64, title: Option<&str>, locator: &str) -> Source {
        Source {
            id,
            book_id: None,
            reader: Reader::Preview,
            locator: locator.into(),
            title: title.map(Into::into),
            author: None,
            ignored: false,
            last_position: None,
            last_seen_at: None,
            collections: Vec::new(),
        }
    }

    #[test]
    fn every_query_word_must_appear_in_any_order() {
        // given
        let text = "Pride and Prejudice, Annotated";

        // when
        let hit = matches("pride prejudice", text);
        let miss = matches("pride anger", text);

        // then
        assert!(hit);
        assert!(!miss);
    }

    #[test]
    fn exact_title_beats_partial_matches() {
        // given
        let books = vec![book(1, "Walden"), book(2, "Walden Companion")];

        // when
        let found = find_books(&books, "walden");

        // then
        assert_eq!(found.iter().map(|b| b.id).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn author_matches_too() {
        // given
        let books = vec![book(1, "Walden")];

        // when
        let found = find_books(&books, "thoreau");

        // then
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn sources_match_on_file_name_when_untitled() {
        // given
        let sources = vec![source(1, None, "/Books/classics/war-and-peace.pdf")];

        // when
        let found = find_sources(&sources, "war peace");

        // then
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn same_title_ignores_subtitles_and_punctuation() {
        // given
        let (books, bookokrat) = (
            "Moby Dick",
            "Moby Dick: Or, The Whale",
        );

        // when
        let same = same_title(books, bookokrat);

        // then
        assert!(same);
    }

    #[test]
    fn empty_query_matches_nothing() {
        // given
        let books = vec![book(1, "Walden")];

        // when
        let found = find_books(&books, "   ");

        // then
        assert!(found.is_empty());
    }
}
