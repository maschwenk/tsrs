// A candidate name shorter than 3 bytes is only considered when it equals the unknown name under strings.EqualFold
// (core.go:614). U+0130 folds only to itself, so 'i' is not suggested for it.
declare const i: number;
İ;
