[Exposed=Window]
interface Foopy {
  constructor();
  readonly attribute USVString foopy;
  Promise<undefined> roundtrip();
};