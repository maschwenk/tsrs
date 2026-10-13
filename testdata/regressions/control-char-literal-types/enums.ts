// A symbol named by a lone DEL is a user name, not an internal one (those have a non-empty rest): keep it raw.
const enum Keys { "\u007f" = 127, "\t" = 9 }
const three: "z" = Keys["\u007f"];
enum Open { "\u007f" = "del", other = "o" }
const four: "z" = Open["\u007f"];
declare const ns: { "\u007f": { inner: 1 } };
const five: "z" = ns;
