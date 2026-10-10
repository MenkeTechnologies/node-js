function f(a, b) { arguments[0] = 7; b = 9; return [a, arguments[1], arguments.length]; }
console.log(f(1, 2), f(1), f());
function g(a) { a = 5; return arguments[0]; }
console.log(g(1), g());
function h(a) { 'use strict'; arguments[0] = 7; a = 3; return [a, arguments[0]]; }
console.log(h(1));
function k(a, a2) { delete arguments[0]; a = 9; return [arguments[0], a]; }
console.log(k(1, 2));
function d(a, a) { arguments[0] = 'x'; arguments[1] = 'y'; return a; }
console.log(d(1, 2));
function c(a) { const cl = () => { a = 11; }; cl(); return arguments[0]; }
console.log(c(1));
function dflt(a, b = 2) { arguments[0] = 9; return a; }
console.log(dflt(1));
function va(a) { var a = 4; return arguments[0]; }
console.log(va(1));
function ev(a) { eval('a = 8'); return arguments[0]; }
console.log(ev(1));
function ar(a) { return (() => { arguments[0] = 6; return a; })(); }
console.log(ar(1));
function sw(a, b) { [].reverse.call(arguments); return [a, b, arguments[0], arguments[1]]; }
console.log(sw(1, 2));
