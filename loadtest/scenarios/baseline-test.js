/**
 * Baseline Load Test for OSM Changeset API
 *
 * Three focused scenarios to establish performance baseline:
 * - Scenario A: Connection contention (mutex bottleneck test)
 * - Scenario B: Memory stress (large result set buffering)
 * - Scenario C: Mixed realistic traffic
 */

import http from 'k6/http';
import { check, sleep } from 'k6';
import { BASE_URL, THRESHOLDS } from '../lib/config.js';
import {
  getRandomChangesetId,
  getWeightedRandomDate,
  getRandomDateRange,
  getRandomHour,
  dateToYYYYMMDD
} from '../lib/utils.js';
import {
  recordResponse,
  extractChangesetCount
} from '../lib/metrics.js';

// Scenario configuration
export const options = {
  scenarios: {
    // Scenario A: Connection Contention Test
    // Tests the Arc<Mutex<Connection>> bottleneck
    // Progressive load to find where mutex serialization causes degradation
    connection_contention: {
      executor: 'ramping-vus',
      exec: 'testConnectionContention',
      startVUs: 0,
      stages: [
        { duration: '1m', target: 10 },   // Warm up
        { duration: '2m', target: 10 },   // Baseline
        { duration: '1m', target: 20 },   // Increase
        { duration: '2m', target: 20 },   // Sustain
        { duration: '1m', target: 40 },   // More pressure
        { duration: '2m', target: 40 },   // Sustain
        { duration: '1m', target: 60 },   // High load
        { duration: '2m', target: 60 },   // Sustain
        { duration: '1m', target: 100 },  // Stress
        { duration: '2m', target: 100 },  // Sustain
        { duration: '1m', target: 0 }     // Ramp down
      ],
      tags: { scenario: 'connection_contention' },
      startTime: '0s'
    },

    // Scenario B: Memory Stress Test
    // Tests memory buffering with large 7-day queries
    // Low concurrency but large result sets
    memory_stress: {
      executor: 'ramping-vus',
      exec: 'testMemoryStress',
      startVUs: 0,
      stages: [
        { duration: '1m', target: 2 },    // Start gentle
        { duration: '3m', target: 2 },    // Sustain
        { duration: '1m', target: 5 },    // Increase
        { duration: '3m', target: 5 },    // Sustain
        { duration: '1m', target: 10 },   // Push limits
        { duration: '3m', target: 10 },   // Sustain (likely to cause issues)
        { duration: '1m', target: 0 }     // Ramp down
      ],
      tags: { scenario: 'memory_stress' },
      startTime: '17m'  // Start after scenario A
    },

    // Scenario C: Mixed Realistic Traffic
    // Simulates production-like request distribution
    // Uses arrival rate (req/sec) instead of VUs
    mixed_traffic: {
      executor: 'ramping-arrival-rate',
      exec: 'testMixedTraffic',
      startRate: 0,
      timeUnit: '1s',
      preAllocatedVUs: 50,
      maxVUs: 200,
      stages: [
        { duration: '1m', target: 10 },   // 10 req/sec
        { duration: '2m', target: 10 },   // Sustain
        { duration: '1m', target: 20 },   // 20 req/sec
        { duration: '2m', target: 20 },   // Sustain
        { duration: '1m', target: 40 },   // 40 req/sec
        { duration: '2m', target: 40 },   // Sustain
        { duration: '1m', target: 60 },   // 60 req/sec (expected bottleneck)
        { duration: '2m', target: 60 },   // Sustain
        { duration: '1m', target: 100 },  // 100 req/sec (stress)
        { duration: '2m', target: 100 },  // Sustain
        { duration: '1m', target: 0 }     // Ramp down
      ],
      tags: { scenario: 'mixed_traffic' },
      startTime: '31m'  // Start after scenario B
    }
  },

  // Overall thresholds - use medium defaults
  thresholds: {
    ...THRESHOLDS.medium,
    'http_req_duration{scenario:connection_contention}': ['p(95)<500', 'p(99)<1000'],
    'http_req_duration{scenario:memory_stress}': ['p(95)<3000', 'p(99)<10000'],
    'http_req_duration{scenario:mixed_traffic}': ['p(95)<1000', 'p(99)<2000']
  },

  // Summary configuration
  summaryTrendStats: ['min', 'avg', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
  summaryTimeUnit: 'ms'
};

/**
 * Scenario A: Connection Contention Test
 * Repeatedly queries single changesets to test mutex contention
 */
export function testConnectionContention() {
  const changesetId = getRandomChangesetId();
  const url = `${BASE_URL}/changeset/${changesetId}.json`;

  const res = http.get(url, {
    tags: { name: 'single_changeset', endpoint_type: 'single' }
  });

  const isSuccess = check(res, {
    'status is 200 or 404': (r) => r.status === 200 || r.status === 404,
    'response time < 1s': (r) => r.timings.duration < 1000
  });

  recordResponse(res, 'single');

  // Very short sleep - we want to hammer the connection
  sleep(0.1);
}

/**
 * Scenario B: Memory Stress Test
 * Queries large date ranges to test memory buffering
 */
export function testMemoryStress() {
  // Get a random 7-day range (maximum allowed)
  const range = getRandomDateRange(7, 'RECENT');  // Focus on recent dates (more data)
  const startDate = dateToYYYYMMDD(range.start);
  const endDate = dateToYYYYMMDD(range.end);

  const url = `${BASE_URL}/changesets/date/from/${startDate}/to/${endDate}.json`;

  const res = http.get(url, {
    tags: { name: 'date_range_7day', endpoint_type: 'range' },
    timeout: '30s'  // Allow longer timeout for large queries
  });

  const isSuccess = check(res, {
    'status is 200': (r) => r.status === 200,
    'response time < 10s': (r) => r.timings.duration < 10000,
    'no timeout': (r) => r.status !== 0
  });

  const changesetCount = extractChangesetCount(res, 'json');
  recordResponse(res, 'range', changesetCount);

  // Longer sleep to avoid overwhelming memory
  sleep(1);
}

/**
 * Scenario C: Mixed Realistic Traffic
 * Simulates production-like request distribution
 */
export function testMixedTraffic() {
  const r = Math.random();
  let res, url, endpointType;

  if (r < 0.2) {
    // 20% Health checks
    url = `${BASE_URL}/health`;
    endpointType = 'health';
    res = http.get(url, { tags: { name: 'health', endpoint_type: endpointType } });

    check(res, {
      'health status is 200': (r) => r.status === 200,
      'health response time < 50ms': (r) => r.timings.duration < 50
    });

  } else if (r < 0.3) {
    // 10% Meta latest (cached)
    url = `${BASE_URL}/meta/latest`;
    endpointType = 'meta';
    res = http.get(url, { tags: { name: 'meta_latest', endpoint_type: endpointType } });

    check(res, {
      'meta status is 200': (r) => r.status === 200,
      'meta response time < 100ms': (r) => r.timings.duration < 100
    });

  } else if (r < 0.6) {
    // 30% Single changeset lookups
    const changesetId = getRandomChangesetId();
    url = `${BASE_URL}/changeset/${changesetId}.json`;
    endpointType = 'single';
    res = http.get(url, { tags: { name: 'single_changeset', endpoint_type: endpointType } });

    check(res, {
      'changeset status is 200 or 404': (r) => r.status === 200 || r.status === 404
    });

  } else if (r < 0.85) {
    // 25% Day queries
    const date = getWeightedRandomDate();
    const dateStr = dateToYYYYMMDD(date);
    url = `${BASE_URL}/changesets/day/${dateStr}.json`;
    endpointType = 'day';
    res = http.get(url, { tags: { name: 'day_query', endpoint_type: endpointType } });

    check(res, {
      'day query status is 200': (r) => r.status === 200
    });

    const changesetCount = extractChangesetCount(res, 'json');
    recordResponse(res, 'day', changesetCount);

  } else if (r < 0.95) {
    // 10% Hour queries
    const date = getWeightedRandomDate();
    const dateStr = dateToYYYYMMDD(date);
    const hour = getRandomHour();
    url = `${BASE_URL}/changesets/day/${dateStr}/hour/${hour}.json`;
    endpointType = 'hour';
    res = http.get(url, { tags: { name: 'hour_query', endpoint_type: endpointType } });

    check(res, {
      'hour query status is 200': (r) => r.status === 200
    });

    const changesetCount = extractChangesetCount(res, 'json');
    recordResponse(res, 'day', changesetCount);

  } else {
    // 5% Small date ranges (1-3 days)
    const range = getRandomDateRange(3);
    const startDate = dateToYYYYMMDD(range.start);
    const endDate = dateToYYYYMMDD(range.end);
    url = `${BASE_URL}/changesets/date/from/${startDate}/to/${endDate}.json`;
    endpointType = 'range';
    res = http.get(url, {
      tags: { name: 'date_range_small', endpoint_type: endpointType },
      timeout: '10s'
    });

    check(res, {
      'range query status is 200': (r) => r.status === 200
    });

    const changesetCount = extractChangesetCount(res, 'json');
    recordResponse(res, 'range', changesetCount);
  }

  // Record metrics if not already done
  if (endpointType === 'health' || endpointType === 'meta' || endpointType === 'single') {
    recordResponse(res, endpointType);
  }

  // Variable sleep based on endpoint type
  if (endpointType === 'health' || endpointType === 'meta') {
    sleep(0.1);  // Fast endpoints - short sleep
  } else if (endpointType === 'single') {
    sleep(0.5);  // Medium sleep
  } else {
    sleep(1);    // Slower endpoints - longer sleep
  }
}

// Default export for simple execution
export default function() {
  // For simple runs without scenarios, do mixed traffic
  testMixedTraffic();
}
