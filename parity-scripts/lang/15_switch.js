// switch with fall-through and grouped cases.
function classify(n) {
  switch (true) {
    case n < 0:
      return "negative";
    case n === 0:
      return "zero";
    case n < 10:
      return "small";
    case n < 100:
      return "medium";
    default:
      return "large";
  }
}

for (const n of [-5, 0, 7, 42, 500]) {
  console.log(n, "->", classify(n));
}

// Intentional fall-through grouping.
function daysInMonth(month) {
  switch (month) {
    case 1:
    case 3:
    case 5:
    case 7:
    case 8:
    case 10:
    case 12:
      return 31;
    case 4:
    case 6:
    case 9:
    case 11:
      return 30;
    case 2:
      return 28;
    default:
      return -1;
  }
}
const months = [];
for (let m = 1; m <= 12; m++) months.push(daysInMonth(m));
console.log("days:", months.join(","));

// Fall-through accumulation.
function benefits(tier) {
  const perks = [];
  switch (tier) {
    case "gold":
      perks.push("priority");
    // fall through
    case "silver":
      perks.push("discount");
    // fall through
    case "bronze":
      perks.push("newsletter");
      break;
    default:
      perks.push("none");
  }
  return perks.join(",");
}
console.log("gold:", benefits("gold"));
console.log("silver:", benefits("silver"));
console.log("bronze:", benefits("bronze"));
