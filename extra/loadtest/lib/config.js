// Load test configuration

export const BASE_URL = __ENV.BASE_URL || 'http://localhost:3000';
export const RESULTS_DIR = __ENV.RESULTS_DIR || './loadtest/results';

// Date ranges for testing
export const DATE_RANGES = {
  RECENT: { start: new Date('2024-01-01'), end: new Date('2025-01-16') },
  MID: { start: new Date('2015-01-01'), end: new Date('2019-12-31') },
  EARLY: { start: new Date('2005-01-01'), end: new Date('2014-12-31') }
};

// Changeset ID range (estimated from exploration)
export const CHANGESET_ID_MAX = 170000000;
export const CHANGESET_ID_MIN = 1;

// Performance thresholds for different endpoint types
export const THRESHOLDS = {
  fast: {
    // For health checks, cached endpoints
    http_req_duration: ['p(95)<50', 'p(99)<100'],
    http_req_failed: ['rate<0.01']
  },
  medium: {
    // For single changeset lookups
    http_req_duration: ['p(95)<500', 'p(99)<1000'],
    http_req_failed: ['rate<0.05']
  },
  slow: {
    // For day queries, range queries
    http_req_duration: ['p(95)<2000', 'p(99)<5000'],
    http_req_failed: ['rate<0.10']
  },
  very_slow: {
    // For user queries (full table scans)
    http_req_duration: ['p(95)<10000', 'p(99)<30000'],
    http_req_failed: ['rate<0.20']
  }
};

// Known active users (for user query testing)
// These are placeholder examples - update with real users from your dataset
export const KNOWN_USERS = [
  'woodpeck',
  'mvexel',
  'fredriklindseth',
  'jumbanho',
  'mapcrafters',
  'Minh Nguyen',
  'manoharuss',
  'NE2',
  'RichRico',
  'stevea'
];

// Common configuration
export const CONFIG = {
  discardResponseBodies: false,  // Keep bodies to measure response size
  noConnectionReuse: false,      // Allow connection reuse
  userAgent: 'k6-loadtest/1.0'
};
