pub const MIN_SEMANTIC_QUERY_CHARS: usize = 2;
pub const MIN_SEMANTIC_SCORE: f32 = 0.42;
pub const HIGH_CONFIDENCE_SEMANTIC_SCORE: f32 = 0.72;

pub fn should_use_semantic_recall(query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.chars().count() < MIN_SEMANTIC_QUERY_CHARS {
        return false;
    }
    trimmed.chars().any(|ch| ch.is_alphabetic())
}

pub fn passes_semantic_score_threshold(score: f32) -> bool {
    score.is_finite() && score >= MIN_SEMANTIC_SCORE
}

pub fn semantic_confidence(score: f32) -> f32 {
    if !passes_semantic_score_threshold(score) {
        return 0.0;
    }
    let span = HIGH_CONFIDENCE_SEMANTIC_SCORE - MIN_SEMANTIC_SCORE;
    if span <= f32::EPSILON {
        return 1.0;
    }
    ((score - MIN_SEMANTIC_SCORE) / span).clamp(0.0, 1.0)
}
