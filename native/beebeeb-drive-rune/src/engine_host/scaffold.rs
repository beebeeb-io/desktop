//! The plan40 scaffold's one helper. Every batch-1 method that is not bound yet hands its answer through
//! [`not_wired`] in its adapter, while its body lives in the slice's own file; a slice binds its method there and
//! removes the `not_wired(…)` around the call in the adapter. That removal is the slice's only edit in the shared
//! adapter file.

/// Hands `answer` back unchanged. The counter reads `not_wired` as a not-wired marker; a slice removes it when it
/// binds.
pub(crate) fn not_wired<T>(answer: T) -> T {
    answer
}
