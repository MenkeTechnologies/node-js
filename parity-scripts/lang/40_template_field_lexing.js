// A `${ … }` field is delimited by LEXING it: a `}` or a quote inside a regex
// literal, a comment, a string or a nested template does not end the field.
const input = "it's";
console.log(`'${input.replace(/'/g, `'\\''`)}'`);
console.log(`${"}"}|${/}/.source}|${/{/.source}|${`}`}`);
console.log(`${ /* } ' */ 5 }`);
console.log(`${ 7 // } `
}`);
console.log(`a${`b${`c${1 + 1}`}`}`);
console.log(`${[1, 2].map((x) => `<${x}>`).join("")}`);
console.log(`${(() => { return `}` })()}`);
console.log(`${{ a: 1 }.a}${{ b: { c: 2 } }.b.c}`);
console.log(`${"`"}${'`'}${/`/.source}`);
console.log(`$${1}$`, `\${1}`, `${1}${2}`);
