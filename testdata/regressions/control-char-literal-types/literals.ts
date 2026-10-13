// Leading DEL next to other controls and non-ASCII text (BMP, astral, separators) in string literal types.
declare const leading: "\u{7F}x" | "\u{7F}\u{0}1" | "\u{7F}\u{85}\u{2028}" | "\u{7F}é" | "\u{7F}😀" | "😀\u{7F}" | "\u{7F}\u{FFFD}";
const one: "z" = leading;

declare const keys: {"b\u{7F}": 2; "😀": 3};
const two: "z" = keys;
