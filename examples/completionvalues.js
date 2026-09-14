// What `eval` RETURNS — the Script completion value. Only a final expression
// statement produced one, so every compound statement answered `undefined`:
// `eval('if(1){2}')`, `eval('{4}')`, `eval('try{5}finally{6}')` and every loop.
//
// The rule is accumulate-and-reset: an expression statement at any depth
// updates the value, and `if`, every loop, `try` and `switch` reset it to
// `undefined` first (their `UpdateEmpty(result, undefined)` step). A block, a
// labelled statement, `;` and every declaration propagate the previous value
// instead.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

show("if           ", () => [eval("if(1){2}"), eval("if(0){2}"), eval("if(0){2}else{3}")]);
show("block        ", () => [eval("{4}"), eval("{}"), eval("{ { 12 } }")]);
show("loops        ", () => [eval("for(let i=0;i<3;i++){i}"), eval("for(let i=0;i<0;i++){i}"), eval("let i=0; while(i<2){i++; 9}"), eval("let j=0; do { 11 } while(++j<1)")]);
show("for-of-in    ", () => [eval("for(const x of [1,2]){x}"), eval("for(const k in {a:1}){k}")]);
show("try          ", () => [eval("try{5}finally{6}"), eval("try{throw 1}catch(e){7}"), eval("try{17}catch(e){18}finally{19}")]);
show("switch       ", () => [eval("switch(1){case 1: 8}"), eval("switch(1){case 1: 12; break; case 2: 13}"), eval("switch(1){case 1: 10; case 2: 11}")]);
show("labelled     ", () => [eval("lbl: { 10 }"), eval("lbl: { 1; break lbl; }")]);
show("declarations ", () => [eval("var v=1"), eval("let l=1"), eval("function fd(){}"), eval("class K{}"), eval(";"), eval("")]);
// The reset is what makes these `undefined` rather than the earlier value.
show("reset        ", () => [eval("1; if(0){2}"), eval("1; if(1){}"), eval("1; while(false){9}"), eval("1; try{}finally{}"), eval("1; switch(9){}"), eval("1; for(const x of []){9}")]);
show("no-reset     ", () => [eval("1; {}"), eval("1; ;"), eval("1; var v2=2"), eval("1; let l2=2"), eval("1; function f2(){}"), eval("1; lbl: {}")]);
// A `break` out of a loop keeps only what the CURRENT iteration produced: the
// `if` around the break resets first, which is why the first is `undefined` and
// the second is 0.
show("break        ", () => [eval("for(let i=0;i<5;i++){ if(i===2) break; i }"), eval("for(let i=0;i<5;i++){ i; break; }"), eval("let k=0; while(k<3){ k++; if(k===2) break; 5 }")]);
show("continue     ", () => eval("for(let i=0;i<3;i++){ if(i===1) continue; 8 }"));
show("nested-break ", () => eval("outer: for(let i=0;i<2;i++){ for(let j=0;j<2;j++){ 7; break outer } }"));
// A nested FUNCTION's statements are not the script's. The calls below happen
// in a DECLARATION initializer, which does not update the value itself — so a
// body that leaked into the register would be the only thing that could change
// the answer.
show("function-body", () => [eval("function f(){ 1; 2; } f(); 3"), eval("(function(){ 7; return 8 })()"), eval("(() => { 9; })()")]);
show("body-isolated", () => [eval("function f(){ 42 } 7; let y = f();"), eval("const g = () => { 43 }; 8; const z = g();")]);
show("expressions  ", () => [eval("1, 2, 3"), eval("let a; a = 14"), eval("let b=0; b++"), eval("1 ? 15 : 16")]);
// A `finally` that completes normally has its completion DISCARDED, and a
// throwing one replaces everything.
show("finally      ", () => { try { return eval("try{1}finally{ throw new Error('f') }"); } catch (e) { return e.message; } });
show("directive    ", () => eval("'use strict'; 20"));
