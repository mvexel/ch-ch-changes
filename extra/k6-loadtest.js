import http, { get } from "k6/http";
import { check, sleep } from "k6";

const BASE = __ENV.BASE_URL || "http://localhost:3000";

export const options = {
  discardResponseBodies: true,
  scenarios: {
    constantvu: {
      executor: "constant-vus",
      vus: 20,
      duration: "5m",
    },
  },
  thresholds: {
    http_req_failed: ["rate<0.01"],
    http_req_duration: ["p(95)<500", "p(99)<1200"],
  },
};

function req(url, name) {
  const res = http.get(url, { tags: { name } });
  if (res.error || res.status === 0 || res.status >= 500) {
    const code = res.error_code ? ` code=${res.error_code}` : "";
    const err = res.error ? ` err=${res.error}` : "";
    console.warn(`fail name=${name} url=${url} status=${res.status}${code}${err}`);
  }
  return res;
}

// get a random year, month, and day between 2006 and 2025 inclusive
function get_random_date() {
  const start = new Date();
  start.setFullYear(2006 + Math.floor(Math.random() * 20));
  start.setMonth(Math.floor(Math.random() * 12));
  start.setDate(Math.floor(Math.random() * 31));
  return start;
}

function get_random_date_range(howmany) {
  const start = get_random_date();
  const end = new Date(start);
  end.setDate(end.getDate() + Math.floor(Math.random() * howmany));
  return { start, end };
}

function date_to_yyyymmdd(date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}${month}${day}`;
}

export default function () {
  const r = Math.random();
  let res;

  const random_changeset_id = Math.floor(Math.random() * 100000000);
  const random_hour = Math.floor(Math.random() * 24);
  const random_minute = Math.floor(Math.random() * 60);
  if (r < 0.1) {
    res = req(`${BASE}/health`, "health");
  // } else if (r < 0.2) {
  //   res = req(`${BASE}/meta/tip`, "meta_tip");
  // } else if (r < 0.35) {
  //   res = req(`${BASE}/meta/coverage`, "meta_coverage");
  } else if (r < 0.2) {
    res = req(`${BASE}/changeset/${random_changeset_id}.json`, "changeset_single");
  } else if (r < 0.8) {
    res = req(`${BASE}/changesets/day/${date_to_yyyymmdd(get_random_date())}`, "changesets_day");
  } else if (r < 0.95) {
    res = req(`${BASE}/changesets/day/${date_to_yyyymmdd(get_random_date())}/hour/${random_hour}/minute/${random_minute}`,
      "changesets_day_minute"
    );
  } else {
    const dates = get_random_date_range(2);
    const start_date = date_to_yyyymmdd(dates.start);
    const end_date = date_to_yyyymmdd(dates.end);
    res = req(`${BASE}/changesets/from/${start_date}/to/${end_date}`, "changesets_range");
  }

  check(res, { "status 200": (r) => r.status === 200 });
  sleep(1);
}
