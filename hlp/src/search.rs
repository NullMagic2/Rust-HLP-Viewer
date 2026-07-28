//! Deterministic in-memory search over decoded WinHelp topics and authored keywords.

use std::collections::{BTreeMap, BTreeSet};

use crate::{KeywordTable, Topic, TopicOffset};

/// One authored keyword after its raw TOPICOFFSET targets have been resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedKeyword {
    pub keyword: String,
    pub topic_indices: Vec<usize>,
}

/// Why a full-text result received its strongest score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SearchMatchKind {
    Body,
    Title,
    Keyword,
}

/// One ranked topic returned by the in-memory search index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub topic_index: usize,
    pub title: String,
    pub score: u32,
    pub match_kind: SearchMatchKind,
}

/// Pre-folded topic and keyword data built once when an HLP is opened.
#[derive(Debug, Clone, Default)]
pub(crate) struct SearchIndex {
    topics: Vec<SearchTopic>,
    keywords: Vec<ResolvedKeyword>,
}

#[derive(Debug, Clone)]
struct SearchTopic {
    title: String,
    folded_title: String,
    folded_body: String,
    folded_keywords: Vec<String>,
}

impl SearchIndex {
    pub(crate) fn build<F>(
        topics: &[Topic],
        keyword_table: Option<&KeywordTable>,
        mut resolve_offset: F,
    ) -> Self
    where
        F: FnMut(TopicOffset) -> Option<usize>,
    {
        let mut resolved_keywords = Vec::new();
        let mut keywords_by_topic: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        if let Some(table) = keyword_table {
            for entry in &table.entries {
                let mut indices: Vec<usize> = entry
                    .topic_offsets
                    .iter()
                    .filter_map(|offset| resolve_offset(*offset))
                    .collect();
                indices.sort_unstable();
                indices.dedup();
                if indices.is_empty() {
                    continue;
                }
                for index in &indices {
                    keywords_by_topic
                        .entry(*index)
                        .or_default()
                        .insert(fold(&entry.keyword));
                }
                resolved_keywords.push(ResolvedKeyword {
                    keyword: entry.keyword.clone(),
                    topic_indices: indices,
                });
            }
        }
        resolved_keywords.sort_by(|left, right| {
            fold(&left.keyword)
                .cmp(&fold(&right.keyword))
                .then_with(|| left.keyword.cmp(&right.keyword))
        });

        let topics = topics
            .iter()
            .enumerate()
            .map(|(index, topic)| SearchTopic {
                title: topic.title.clone(),
                folded_title: fold(&topic.title),
                folded_body: fold(&topic.plain_text),
                folded_keywords: keywords_by_topic
                    .remove(&index)
                    .map(|items| items.into_iter().collect())
                    .unwrap_or_default(),
            })
            .collect();

        Self {
            topics,
            keywords: resolved_keywords,
        }
    }

    pub(crate) fn keywords(&self) -> &[ResolvedKeyword] {
        &self.keywords
    }

    /// Ranks exact/prefix authored metadata above body-text occurrence while keeping
    /// tie-breaking deterministic in topic order.
    pub(crate) fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let query = fold(query.trim());
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let query_terms: Vec<&str> = query.split_whitespace().collect();
        let mut hits = Vec::new();
        for (topic_index, topic) in self.topics.iter().enumerate() {
            let mut score = 0_u32;
            let mut kind = SearchMatchKind::Body;

            if topic.folded_title == query {
                score = score.max(1_000);
                kind = SearchMatchKind::Title;
            } else if topic.folded_title.starts_with(&query) {
                score = score.max(850);
                kind = SearchMatchKind::Title;
            } else if topic.folded_title.contains(&query) {
                score = score.max(700);
                kind = SearchMatchKind::Title;
            }

            for keyword in &topic.folded_keywords {
                let keyword_score = if keyword == &query {
                    950
                } else if keyword.starts_with(&query) {
                    800
                } else if keyword.contains(&query) {
                    650
                } else {
                    0
                };
                if keyword_score > score {
                    score = keyword_score;
                    kind = SearchMatchKind::Keyword;
                }
            }

            if topic.folded_body.contains(&query) {
                let body_score = 400_u32.saturating_add(
                    u32::try_from(query.len().min(100)).unwrap_or(100),
                );
                if body_score > score {
                    score = body_score;
                    kind = SearchMatchKind::Body;
                }
            } else if query_terms.len() > 1
                && query_terms.iter().all(|term| topic.folded_body.contains(term))
            {
                let body_score = 300_u32.saturating_add(
                    u32::try_from(query_terms.len().min(20) * 5).unwrap_or(100),
                );
                if body_score > score {
                    score = body_score;
                    kind = SearchMatchKind::Body;
                }
            }

            if score != 0 {
                hits.push(SearchHit {
                    topic_index,
                    title: topic.title.clone(),
                    score,
                    match_kind: kind,
                });
            }
        }
        hits.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.topic_index.cmp(&right.topic_index))
        });
        hits.truncate(limit);
        hits
    }
}

fn fold(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| if character.is_whitespace() { ' ' } else { character })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeywordEntry, TopicId, TopicMetadata};

    fn topic(id: i32, title: &str, plain_text: &str) -> Topic {
        Topic {
            id: TopicId(crate::TopicPos(id)),
            title: title.to_owned(),
            macros: Vec::new(),
            metadata: TopicMetadata::default(),
            non_scrolling: Vec::new(),
            scrolling: Vec::new(),
            unclassified: Vec::new(),
            plain_text: plain_text.to_owned(),
        }
    }

    #[test]
    fn title_and_keyword_rank_above_body_text() {
        let topics = vec![
            topic(1, "Installing", "ordinary setup discussion"),
            topic(2, "Other", "installing appears in body"),
        ];
        let table = KeywordTable {
            id: 'K',
            entries: vec![KeywordEntry {
                keyword: "Install".to_owned(),
                topic_offsets: vec![TopicOffset(10)],
                has_macro_target: false,
            }],
        };
        let index = SearchIndex::build(&topics, Some(&table), |offset| {
            (offset == TopicOffset(10)).then_some(0)
        });
        let hits = index.search("install", 10);
        assert_eq!(hits[0].topic_index, 0);
        assert_eq!(hits[0].match_kind, SearchMatchKind::Keyword);
        assert_eq!(hits[1].topic_index, 1);
    }

    #[test]
    fn multiword_search_accepts_all_terms_without_exact_phrase() {
        let topics = vec![topic(1, "Networking", "configure the adapter before network setup")];
        let index = SearchIndex::build(&topics, None, |_| None);
        assert_eq!(index.search("adapter setup", 10).len(), 1);
    }
}
