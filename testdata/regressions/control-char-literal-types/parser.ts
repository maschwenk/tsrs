// A union of readonly tuples of strings that start with U+007F (DEL) and other controls: the type text must keep
// DEL raw (like tsgo) and stay under the 320-character diagnostic truncation limit.
declare function parse(chunks: string[]): string[];

const cases = [
	{chunks: ['\u{8}']},
	{chunks: ['\u{8}\u{8}\u{8}']},
	{chunks: ['\u{8}\u{1B}[A\u{8}']},
	{chunks: ['\ttest']},
	{chunks: ['\rtest']},
	{chunks: ['\u{1B}[A\u{7F}\u{1B}[B']},
	{chunks: ['\u{1B}[A\u{7F}\u{7F}']},
	{chunks: ['abc\u{7F}']},
	{chunks: ['abc\u{7F}\u{7F}\u{7F}']},
	{chunks: ['ab\u{7F}cd']},
	{chunks: ['\u{7F}']},
	{chunks: ['\u{7F}\u{8}\u{7F}']},
	{chunks: ['\u{7F}\u{7F}\u{1B}[A']},
	{chunks: ['\u{7F}\u{7F}\u{7F}']},
] as const;

for (const testCase of cases) {
	parse(testCase.chunks);
}
