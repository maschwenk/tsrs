#!/usr/bin/env node
// Generate directly from the same Unicode data as TypeScript's generate-unicode-data.mts.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { parseArgs } from 'node:util';

const require = createRequire(import.meta.url);
const unicode = '@unicode/unicode-15.1.0';
const load = property => require(`${unicode}/${property}/code-points.js`);
const { values } = parseArgs({ options: { check: { type: 'boolean' }, 'verify-go': { type: 'string' } } });
const destination = new URL('../crates/tsrs_core/src/stringutil/js_case_generated.rs', import.meta.url);

const simpleLower = load('Simple_Case_Mapping/Lowercase');
const simpleUpper = load('Simple_Case_Mapping/Uppercase');
const fullLower = load('Special_Casing/Lowercase');
const fullUpper = load('Special_Casing/Uppercase');
const finalSigma = load('Special_Casing/Lowercase--Final_Sigma');
const keys = [...new Set([...simpleLower.keys(), ...simpleUpper.keys(), ...fullLower.keys(), ...fullUpper.keys(), ...finalSigma.keys()])].sort((a, b) => a - b);
const entries = keys.map(key => ({
    key,
    lower: String.fromCodePoint(...(fullLower.get(key) ?? [simpleLower.get(key) ?? key])),
    upper: String.fromCodePoint(...(fullUpper.get(key) ?? [simpleUpper.get(key) ?? key])),
    conditionalLower: String.fromCodePoint(...(finalSigma.get(key) ?? [])),
    condition: finalSigma.has(key) ? 'FinalSigma' : 'None',
}));

// Match the upstream stride grouping, splitting BMP and astral code points first.
function strideRanges(points) {
    const ranges = [];
    for (let i = 0; i < points.length;) {
        const lo = points[i];
        const stride = i + 1 < points.length ? points[i + 1] - lo : 1;
        let hi = lo;
        while (++i < points.length && points[i] === hi + stride) hi = points[i];
        ranges.push({ lo, hi, stride });
    }
    return ranges;
}

function rangeTable(property) {
    const points = [...new Set(load(`Binary_Property/${property}`))].sort((a, b) => a - b);
    const r16 = strideRanges(points.filter(cp => cp <= 0xFFFF));
    const r32 = strideRanges(points.filter(cp => cp > 0xFFFF));
    const latinOffset = r16.filter(range => range.hi <= 0xFF).length;
    assert.deepEqual([...r16, ...r32].flatMap(({ lo, hi, stride }) =>
        Array.from({ length: (hi - lo) / stride + 1 }, (_, i) => lo + i * stride)), points);
    for (const { lo, hi, stride } of r16) assert(lo <= 0xFFFF && hi <= 0xFFFF && stride <= 0xFFFF);
    return { r16, r32, latinOffset };
}

const cased = rangeTable('Cased');
const caseIgnorable = rangeTable('Case_Ignorable');

// The Go file is an optional independent oracle, never an input to generation.
if (values['verify-go']) {
    const go = readFileSync(values['verify-go'], 'utf8').replaceAll('\r\n', '\n');
    const decode = text => text.replace(/\\(?:u([\dA-Fa-f]{4})|U([\dA-Fa-f]{8}))/g,
        (_, short, long) => String.fromCodePoint(parseInt(short ?? long, 16)));
    const original = [...go.matchAll(/\t(0x[\dA-F]+):\s+\{lower: "([^"]*)", upper: "([^"]*)"(?:, conditionalLower: "([^"]*)")?, condition: specialCasingCondition(\w+)\}/g)]
        .map(([, key, lower, upper, conditionalLower = '', condition]) => ({
            key: Number(key), lower: decode(lower), upper: decode(upper), conditionalLower: decode(conditionalLower), condition,
        }));
    assert.deepEqual(entries, original, 'Unicode mappings differ from the pinned Go table');
    for (const [name, table] of [['unicodeCasedRanges', cased], ['unicodeCaseIgnorableRanges', caseIgnorable]]) {
        const body = go.match(new RegExp(`var ${name} = &unicode\\.RangeTable\\{([\\s\\S]*?)\\n\\}`))[1];
        const ranges = kind => [...body.match(new RegExp(`${kind}: \\[\\]unicode\\.Range\\d+\\{([\\s\\S]*?)\\n\\t\\}`))[1]
            .matchAll(/\{(0x[\dA-Fa-f]+), (0x[\dA-Fa-f]+), (\d+)\}/g)]
            .map(([, lo, hi, stride]) => ({ lo: Number(lo), hi: Number(hi), stride: Number(stride) }));
        assert.deepEqual(table, { r16: ranges('R16'), r32: ranges('R32'), latinOffset: Number(body.match(/LatinOffset: (\d+)/)[1]) }, name);
    }
}

const strings = new Map();
let pool = '';
let poolLength = 0;
function intern(text) {
    if (!strings.has(text)) {
        const length = Buffer.byteLength(text);
        assert(poolLength <= 0xFFFF, 'casing offset exceeds u16');
        assert(length <= 0xFF, 'casing length exceeds u8');
        strings.set(text, { offset: poolLength, length });
        pool += text;
        poolLength += length;
        assert(poolLength <= 0x10000, 'casing pool exceeds u16 address space');
    }
    return strings.get(text);
}
intern('');
const records = entries.map(entry => ({ ...entry, spans: [entry.lower, entry.upper, entry.conditionalLower].map(intern) }));
const bytes = Buffer.from(pool);
for (const { lower, upper, conditionalLower, spans } of records) {
    for (const [i, text] of [lower, upper, conditionalLower].entries()) {
        const { offset, length } = spans[i];
        assert(offset + length <= bytes.length);
        assert(bytes.subarray(offset, offset + length).equals(Buffer.from(text)), 'casing string did not round-trip');
    }
}

const hex = n => `0x${n.toString(16).toUpperCase()}`;
const rustString = text => `"${[...text].map(c => `\\u{${c.codePointAt(0).toString(16).toUpperCase()}}`).join('')}"`;
const poolPoints = [...pool];
const poolLines = [];
for (let i = 0; i < poolPoints.length; i += 16) poolLines.push(`    ${rustString(poolPoints.slice(i, i + 16).join(''))},\n`);
function renderRanges(name, table) {
    return ['r16', 'r32'].map(kind => {
        const type = kind === 'r16' ? 'Range16' : 'Range32';
        return `static ${name}_${kind.toUpperCase()}: &[${type}] = &[\n${table[kind].map(({ lo, hi, stride }) =>
            `    ${type} { lo: ${hex(lo)}, hi: ${hex(hi)}, stride: ${stride} },\n`).join('')}];\n`;
    }).join('') + `pub(crate) static ${name}: RangeTable = RangeTable { r16: ${name}_R16, r32: ${name}_R32, latin_offset: ${table.latinOffset} };\n\n`;
}

const output = `// Code generated by tasks/gen-stringutil.mjs from ${unicode}@${require(`${unicode}/package.json`).version}. DO NOT EDIT.

use super::rangetable::{Range16, Range32, RangeTable};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum SpecialCasingCondition {
    None,
    FinalSigma,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SpecialCasingMapping {
    pub offsets: [u16; 3],
    pub lengths: [u8; 3],
    pub condition: SpecialCasingCondition,
}

const _: () = assert!(std::mem::size_of::<(u32, SpecialCasingMapping)>() == 16);

// UTF-8 byte offsets and lengths, in lower / upper / conditional-lower order.
// ${strings.size} distinct strings, ${poolLength} bytes; generation validates every slice.
pub(crate) static CASING_STRINGS: &str = concat!(
${poolLines.join('')});

// Sorted by code point; looked up with binary search (Go uses a map).
pub(crate) static SPECIAL_CASING_MAPPINGS: &[(u32, SpecialCasingMapping)] = &[
${records.map(({ key, spans, condition }) => `    (${hex(key)}, SpecialCasingMapping { offsets: [${spans.map(s => s.offset).join(', ')}], lengths: [${spans.map(s => s.length).join(', ')}], condition: SpecialCasingCondition::${condition} }),\n`).join('')}];

${renderRanges('UNICODE_CASED_RANGES', cased)}${renderRanges('UNICODE_CASE_IGNORABLE_RANGES', caseIgnorable)}`;

if (values.check) {
    assert(readFileSync(destination, 'utf8') === output, 'stale casing table: run node tasks/gen-stringutil.mjs');
} else {
    writeFileSync(destination, output);
}
console.log(`${entries.length} casing entries; ${poolLength} pooled UTF-8 bytes${values['verify-go'] ? '; all mappings and ranges match Go' : ''}`);
