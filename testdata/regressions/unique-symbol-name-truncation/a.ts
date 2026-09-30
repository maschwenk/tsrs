// The node builder counts a property's internal name toward the truncation budget. For a property keyed by a
// well-known symbol that name embeds the symbol id ("\xFE@iterator@<id>"), so the id's digit count decides where
// the message is cut: symbol ids must be assigned when Go assigns them (valueSymbolLinks is keyed by id).
const o = {
  appppppppppppppppppppppppppppppppppppppppppppppppppp: 1,
  [Symbol.iterator]: 1, [Symbol.asyncIterator]: 1, [Symbol.hasInstance]: 1, [Symbol.isConcatSpreadable]: 1,
  [Symbol.match]: 1, [Symbol.replace]: 1, [Symbol.search]: 1, [Symbol.species]: 1, [Symbol.split]: 1,
  [Symbol.toPrimitive]: 1, [Symbol.toStringTag]: 1, [Symbol.unscopables]: 1, b: 1, c: 1, d: 1, e: 1,
};
const n: number = o;
