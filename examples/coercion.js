// Coercion, equality and typeof — the conversions that surprise people, pinned
// so a change to `+`, `==` or ToPrimitive cannot quietly alter them.
const show = (l, v) => console.log(l, typeof v === 'string' ? JSON.stringify(v) : String(v));
show('arr+arr  ', [] + []);
show('arr+obj  ', [] + {});
show('obj+arr  ', ({}) + []);
show('1+"2"    ', 1 + '2');
show('"3"*"4"  ', '3' * '4');
show('true+true', true + true);
show('null+1   ', null + 1);
show('undef+1  ', undefined + 1);
show('[]==false', [] == false);
show('"0"==false', '0' == false);
show('null==undef', null == undefined);
show('null>=0  ', null >= 0);
show('NaN==NaN ', NaN == NaN);
show('typeof-fn', typeof function(){});
show('typeof-null', typeof null);
show('typeof-sym', typeof Symbol());
show('0.1+0.2  ', 0.1 + 0.2);
show('-0===0   ', -0 === 0);
show('Object.is', Object.is(-0, 0));
show('1/-0     ', 1 / -0);
show('2**53+1  ', 2 ** 53 + 1);
show('parseInt ', parseInt('08'), parseInt('0x10'));
show('Number("")', Number(''));
show('+" 12 "  ', +' 12 ');
show('[1,2]=="1,2"', [1, 2] == '1,2');
show('valueOf  ', +{ valueOf() { return 7; } });
show('toString ', `${{ toString() { return 'T'; } }}`);
show('sortDflt ', [10, 9, 1].sort().join(','));
show('sparse   ', JSON.stringify([1, , 3]));
show('holes-map', [1, , 3].map(x => x * 2).length);

// OrdinaryToPrimitive (7.1.1.1) throws when neither `valueOf` nor `toString`
// yields a primitive. The `[object Tag]` fallback here is for exotics whose
// property funnel exposes no callable `toString` — it was also catching an
// object whose OWN methods ran and returned non-primitives, so this quietly
// produced "[object Object]1" instead of throwing.
const noPrimitive = { valueOf() { return {}; }, toString() { return {}; } };
const caught = (f) => { try { return f(); } catch (e) { return e.constructor.name; } };
console.log("noprim  ", caught(() => noPrimitive + 1), caught(() => +noPrimitive), caught(() => `${noPrimitive}`));
console.log("noprim2 ", caught(() => String(noPrimitive)), caught(() => noPrimitive < 1));
// One usable method is enough, and the hint picks which is tried first.
const halfPrimitive = { valueOf() { return "V"; }, toString() { return {}; } };
console.log("halfprim", `${halfPrimitive}`, halfPrimitive + "");
// A null-prototype object has neither method and has always thrown.
console.log("nullproto", caught(() => Object.create(null) + ""));
// The exotics still get their brand rather than a TypeError.
console.log("exotics ", String(new Map()), String(new Set()), String(Promise.resolve()));
console.log("ordinary", String({}), String([1, 2]), String(/a/g), typeof (new Date(0) - 1));
