// Annex B String.prototype HTML methods (CreateHTML): tag, optional attribute,
// `"` escaped as &quot; and nothing else, value ToString'd even when absent.
const s = "a<b>&";
console.log(s.anchor('x"y'), s.big(), s.blink(), s.bold(), s.fixed());
console.log(s.fontcolor("red"), s.fontsize(7), s.italics(), s.link("http://e.x/?a=1&b=\"2\""));
console.log(s.small(), s.strike(), s.sub(), s.sup());
console.log("x".anchor(), "x".link(null), "x".fontsize({ toString() { return "+1"; } }), "".bold());
console.log(String.prototype.anchor.call(12, 3), String.prototype.sub.call(true));
console.log(String.prototype.anchor.length, String.prototype.big.length, String.prototype.fontcolor.name);
try { String.prototype.big.call(null); } catch (e) { console.log(e.name, e.message); }
try { String.prototype.anchor.call(undefined, "n"); } catch (e) { console.log(e.name, e.message); }
try { "x".anchor(Symbol("s")); } catch (e) { console.log(e.name, e.message); }
// Every String.prototype method is generic over a coercible receiver.
const S = String.prototype;
console.log(S.trim.call(12), S.padEnd.call(true, 6, "-"), S.at.call(123, -1), S.split.call(1.5, "."));
console.log(S.includes.call({ toString() { return "hay"; } }, "a"), S.toUpperCase.call([1, "b"]), S.slice.call(-0, 0), S.repeat.call(7, 2));
console.log([...S[Symbol.iterator].call(42)], S.localeCompare.call(1, "1"), S.concat.call(null === 0, 1));
try { S.trim.call(Symbol("x")); } catch (e) { console.log(e.name, e.message); }
try { S.at.call({ toString() { throw new RangeError("ts"); } }); } catch (e) { console.log(e.name, e.message); }
