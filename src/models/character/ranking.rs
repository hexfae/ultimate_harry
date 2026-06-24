//! Similarity ranking for [`Character`]: the name/nickname fuzzy search that
//! powers the character pickers, filtered to visible or deleted characters.
//!
//! Split out from the model in `character.rs`; as a child module this can still
//! reach `Character`'s private fields.

use super::Character;
use crate::constants::MAX_RESULTS;

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the similarity ranking is split into this child module to separate search from the Character model"
)]
impl Character {
    /// Filters out deleted and superseded characters, then returns up to [`MAX_RESULTS`] ranked by
    /// name (and nickname) similarity to the input, breaking ties by number of conversations had.
    ///
    /// This replaces the old `MOST_SIMILAR_TO` `SurrealQL` query and sets each returned
    /// character's `similarity` for display.
    #[must_use]
    pub fn rank_by_similarity(characters: Vec<Self>, input: &str) -> Vec<Self> {
        Self::rank_filtered(characters, input, Self::is_visible)
    }

    /// Like [`rank_by_similarity`](Self::rank_by_similarity), but ranks only the
    /// soft-deleted characters, used by the restore command to find a character
    /// to bring back from deletion.
    #[must_use]
    pub fn rank_deleted_by_similarity(characters: Vec<Self>, input: &str) -> Vec<Self> {
        Self::rank_filtered(characters, input, Self::is_deleted)
    }

    /// Keeps the characters for which `keep` returns true, then returns up to
    /// [`MAX_RESULTS`] ranked by name (and nickname) similarity to the input,
    /// breaking ties by number of conversations had, setting each returned
    /// character's `similarity` for display.
    fn rank_filtered(
        characters: Vec<Self>,
        input: &str,
        keep: impl Fn(&Self) -> bool,
    ) -> Vec<Self> {
        let mut ranked: Vec<Self> = characters
            .into_iter()
            .filter(|character| keep(character))
            .map(|mut character| {
                let name_similarity =
                    strsim::normalized_damerau_levenshtein(&character.name, input);
                let similarity = character.nickname.as_deref().map_or(
                    name_similarity,
                    |nickname| {
                        name_similarity
                            .max(strsim::normalized_damerau_levenshtein(nickname, input))
                    },
                );
                character.similarity = Some(similarity);
                character
            })
            .collect();
        ranked.sort_by(|left, right| {
            let left_similarity = left.similarity.unwrap_or_default();
            let right_similarity = right.similarity.unwrap_or_default();
            right_similarity
                .total_cmp(&left_similarity)
                .then_with(|| right.conversations_had.cmp(&left.conversations_had))
        });
        ranked.truncate(MAX_RESULTS);
        ranked
    }
}

/// Tests for the character similarity ranking.
#[cfg(test)]
mod tests {
    use super::super::Character;
    use crate::constants::MAX_RESULTS;
    use serenity::all::UserId;

    /// Builds a minimal visible character with the given ID and name.
    fn basic_character(id: &str, name: &str) -> Character {
        Character::builder()
            .id(id.to_owned())
            .name(name)
            .greeting("hello")
            .creator(UserId::new(1))
            .build()
    }

    /// Returns the index of the character with the given ID in the ranked list.
    fn position_of(ranked: &[Character], id: &str) -> Option<usize> {
        ranked.iter().position(|character| character.id() == id)
    }

    /// Ranking filters deleted and superseded characters, orders by similarity (honoring the
    /// nickname), and breaks ties by number of conversations had.
    #[test]
    fn ranks_visible_characters_by_similarity_and_conversations() {
        let exact = basic_character("id-exact", "Banana");
        let close = basic_character("id-close", "Bananas");
        let nickname = Character::builder()
            .id("id-nickname".to_owned())
            .name("Xyzzy")
            .greeting("hello")
            .creator(UserId::new(1))
            .nickname("Banan".to_owned())
            .build();
        let far = basic_character("id-far", "Zzzzzz");

        let mut deleted = basic_character("id-deleted", "Banana");
        deleted.mark_deleted(UserId::new(2));
        let mut superseded = basic_character("id-superseded", "Banana");
        superseded.set_next_version("id-exact".to_owned());

        let tie_high = Character::builder()
            .id("id-tie-high".to_owned())
            .name("Tie")
            .greeting("hello")
            .creator(UserId::new(1))
            .conversations_had(9_u32)
            .build();
        let tie_low = Character::builder()
            .id("id-tie-low".to_owned())
            .name("Tie")
            .greeting("hello")
            .creator(UserId::new(1))
            .conversations_had(1_u32)
            .build();

        let ranked = Character::rank_by_similarity(
            vec![
                exact, close, nickname, far, deleted, superseded, tie_high, tie_low,
            ],
            "Banana",
        );

        assert_eq!(
            ranked.len(),
            6,
            "deleted and superseded characters are filtered out"
        );
        assert!(
            position_of(&ranked, "id-deleted").is_none(),
            "a deleted character never appears in the ranking"
        );
        assert!(
            position_of(&ranked, "id-superseded").is_none(),
            "a superseded character never appears in the ranking"
        );
        assert_eq!(
            ranked.first().map(Character::name),
            Some("Banana"),
            "the exact name match ranks first"
        );
        let nickname_at = position_of(&ranked, "id-nickname");
        let far_at = position_of(&ranked, "id-far");
        assert!(
            nickname_at.is_some() && far_at.is_some(),
            "both the nickname and the dissimilar character are ranked"
        );
        assert!(
            nickname_at < far_at,
            "a close nickname outranks a dissimilar name"
        );
        let tie_high_at = position_of(&ranked, "id-tie-high");
        let tie_low_at = position_of(&ranked, "id-tie-low");
        assert!(
            tie_high_at.is_some() && tie_low_at.is_some(),
            "both tied characters are ranked"
        );
        assert!(
            tie_high_at < tie_low_at,
            "equal similarity breaks ties toward more conversations"
        );
    }

    /// Deleted-character ranking filters to only deleted characters and orders
    /// them by name similarity to the input.
    #[test]
    fn rank_deleted_by_similarity_returns_only_deleted_characters() {
        let visible = basic_character("id-visible", "Banana");
        let mut deleted_match = basic_character("id-deleted", "Banana");
        deleted_match.mark_deleted(UserId::new(2));
        let mut deleted_other = basic_character("id-deleted-other", "Zzzzzz");
        deleted_other.mark_deleted(UserId::new(2));

        let ranked = Character::rank_deleted_by_similarity(
            vec![visible, deleted_match, deleted_other],
            "Banana",
        );

        assert!(
            position_of(&ranked, "id-visible").is_none(),
            "a visible character never appears among deleted results"
        );
        assert_eq!(
            ranked.first().map(Character::id),
            Some("id-deleted"),
            "the closest-matching deleted character ranks first"
        );
        assert_eq!(ranked.len(), 2, "only deleted characters are returned");
    }

    /// `rank_by_similarity` caps its output at the shared `MAX_RESULTS`, matching the database
    /// listing queries and Discord's 25-option select-menu limit.
    #[test]
    fn ranking_is_capped_at_max_results() {
        let characters = (0..MAX_RESULTS.saturating_add(5))
            .map(|index| basic_character(&format!("id-{index}"), "Banana"))
            .collect();

        let ranked = Character::rank_by_similarity(characters, "Banana");

        assert_eq!(
            ranked.len(),
            MAX_RESULTS,
            "the ranking never returns more than MAX_RESULTS characters"
        );
    }
}
