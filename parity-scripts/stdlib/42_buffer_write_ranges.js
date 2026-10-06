// Buffer integer/float/BigInt writers: value range (checkInt), offset bounds,
// byteLength validation; Buffer.concat list/length validation and truncation.
const b = Buffer.alloc(8);
const cases = [["writeUInt8",300],["writeUInt8",-1],["writeInt8",128],["writeInt8",-129],["writeUInt16LE",65536],["writeInt16BE",-32769],["writeUInt32LE",2**32],["writeInt32LE",2**31],["writeUInt8",1.5],["writeUInt8","7"],["writeUInt8",NaN],["writeUIntLE",2**48,0,6],["writeIntBE",2**47,0,6],["writeUIntLE",256,0,1],["writeUInt8",1,8],["writeUInt8",1,-1],["writeUInt8",1,1.5],["writeUInt16LE",1,7],["writeDoubleLE",1,1],["writeFloatBE",1e40],["writeBigInt64LE",1],["writeBigUInt64LE",-1n],["writeBigInt64LE",2n**63n]];
for (const [m, ...a] of cases) { try { const r = b[m](...a); console.log(m, a.map(String), "ok", r, b.toString("hex")) } catch (e) { console.log(m, a.map(String), e.code, e.message) } }
for (const [m, ...a] of [["readUInt8",8],["readUInt16LE",7],["readInt32BE",-1],["readUInt8",1.5],["readUIntLE",0,7],["readDoubleLE",1]]) { try { console.log(m, b[m](...a)) } catch (e) { console.log(m, e.code, e.message) } }
console.log(Buffer.concat([Buffer.from("a"),Buffer.from("bc")],2).toString(), Buffer.concat([Buffer.from("a")],3), Buffer.concat([], 0).length);
const a2=Buffer.from("ab"), b2=Buffer.from("cd");
console.log(Buffer.concat([a2,b2],3), Buffer.concat([a2,b2],6), Buffer.concat([a2,b2],0), Buffer.concat([new Uint8Array([9]), a2]));
for (const args of [[[a2],-1],[[a2],"2"],[[1]],["x"],[[a2],NaN],[[a2],1.7],[[a2],2**53]]) { try { console.log(Buffer.concat(...args)) } catch(e) { console.log(e.code, e.message) } }
