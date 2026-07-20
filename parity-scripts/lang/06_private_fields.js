// Private #fields and #methods with encapsulation.
class BankAccount {
  #balance = 0;
  #history = [];

  constructor(initial) {
    this.#balance = initial;
    this.#log("open", initial);
  }

  #log(op, amt) {
    this.#history.push(`${op}:${amt}`);
  }

  deposit(amt) {
    this.#balance += amt;
    this.#log("deposit", amt);
    return this;
  }

  withdraw(amt) {
    if (amt > this.#balance) throw new Error("insufficient funds");
    this.#balance -= amt;
    this.#log("withdraw", amt);
    return this;
  }

  get balance() {
    return this.#balance;
  }

  get log() {
    return this.#history.join(" | ");
  }

  static isAccount(obj) {
    return #balance in obj;
  }
}

const acct = new BankAccount(100);
acct.deposit(50).withdraw(30).deposit(10);
console.log("balance:", acct.balance);
console.log("history:", acct.log);
console.log("isAccount(acct):", BankAccount.isAccount(acct));
console.log("isAccount({}):", BankAccount.isAccount({}));
try {
  acct.withdraw(1000);
} catch (e) {
  console.log("caught:", e.message);
}
