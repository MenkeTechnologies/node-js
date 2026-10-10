var o = { a: 1, b: 2, f() { return this === o; } };
var a = 'outer', c = 'c';
with (o) {
  console.log(a, b, c, typeof a, typeof zzz);
  a = 10; c = 'C2';
  var d = 5;
  console.log(f(), (() => a)());
  var a = 77;
}
console.log(o.a, c, d, a);
with ({ x: 1 }) { var x = 2; }
console.log(x);
with ([1, 2, 3]) { console.log(length, typeof push, join('-')); }
with ('abc') { console.log(length, toUpperCase()); }
try { with (null) {} } catch (e) { console.log(e.constructor.name, e.message); }
var p = { v: 1 };
function g() { with (p) { v++; v += 2; return function () { return v; }; } }
var h = g(); p.v = 100; console.log(h(), p.v);
with ({ [Symbol.unscopables]: { hid: true }, hid: 5, vis: 6 }) { var hid = 'x'; console.log(typeof vis, typeof hid, hid); }
console.log(hid);
with (Math) { console.log(max(1, 2), PI > 3, floor(2.5)); }
function k(arg) { with (arg) { return z; } }
try { k({}); } catch (e) { console.log(e.name + ': ' + e.message); }
label: with ({}) { break label; }
for (var i = 0; i < 3; i++) { with ({ i2: i }) { if (i2 == 1) continue; console.log(i2); } }
var q = { a: 1 };
with (q) { console.log(eval('a'), eval('a = 5; a'), q.a); eval('var qq = 9'); }
console.log(qq);
function s() { 'use strict'; try { eval('with ({}) {}'); } catch (e) { console.log(e.name, e.message); } }
s();
try { eval('"use strict"; with ({}) {}'); } catch (e) { console.log(e.name, e.message); }
console.log(new Function('o', 'with (o) { return x + 1 }')({ x: 4 }));
with ({ g: function () { return typeof this; } }) console.log(g());
