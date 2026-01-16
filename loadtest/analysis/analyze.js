#!/usr/bin/env node

/**
 * K6 Load Test Results Analyzer
 *
 * Processes k6 JSON output and generates:
 * - CSV file for spreadsheet analysis
 * - Summary text file with key metrics
 */

const fs = require('fs');
const readline = require('readline');
const path = require('path');

// Check command line arguments
if (process.argv.length < 3) {
  console.error('Usage: node analyze.js <k6-json-output-file> [output-prefix]');
  console.error('Example: node analyze.js results/json/baseline.json baseline');
  process.exit(1);
}

const inputFile = process.argv[2];
const outputPrefix = process.argv[3] || path.basename(inputFile, '.json');

// Output paths
const csvFile = path.join(path.dirname(inputFile), '..', 'csv', `${outputPrefix}.csv`);
const summaryFile = path.join(path.dirname(inputFile), '..', `${outputPrefix}-summary.txt`);

// Data structures for analysis
const metrics = {
  http_req_duration: [],
  http_req_waiting: [],
  http_req_connecting: [],
  http_req_sending: [],
  http_req_receiving: []
};

const metricsByEndpoint = {};
const metricsByScenario = {};
const errors = [];
let totalRequests = 0;
let successfulRequests = 0;
let failedRequests = 0;

// CSV data
const csvRows = [];
csvRows.push('timestamp,metric,value,endpoint,scenario,status,error');

console.log(`Analyzing ${inputFile}...`);

// Create read stream
const fileStream = fs.createReadStream(inputFile);
const rl = readline.createInterface({
  input: fileStream,
  crlfDelay: Infinity
});

// Process each line
rl.on('line', (line) => {
  try {
    const entry = JSON.parse(line);

    if (entry.type === 'Point') {
      const metric = entry.metric;
      const value = entry.data.value;
      const tags = entry.data.tags || {};
      const time = new Date(entry.data.time).toISOString();

      // Extract tags
      const endpoint = tags.name || tags.endpoint_type || 'unknown';
      const scenario = tags.scenario || 'unknown';
      const status = tags.status || '';
      const error = tags.error || '';

      // Add to CSV
      csvRows.push(`${time},${metric},${value},${endpoint},${scenario},${status},${error}`);

      // Collect metrics for summary
      if (metric === 'http_req_duration') {
        metrics.http_req_duration.push(value);

        // By endpoint
        if (!metricsByEndpoint[endpoint]) {
          metricsByEndpoint[endpoint] = [];
        }
        metricsByEndpoint[endpoint].push(value);

        // By scenario
        if (!metricsByScenario[scenario]) {
          metricsByScenario[scenario] = [];
        }
        metricsByScenario[scenario].push(value);
      }

      // Track other timing metrics
      if (metrics[metric]) {
        metrics[metric].push(value);
      }

      // Track errors
      if (metric === 'http_req_failed' && value === 1) {
        errors.push({ time, endpoint, scenario, status, error });
        failedRequests++;
      }

      // Count requests
      if (metric === 'http_reqs') {
        totalRequests += value;
      }
    }
  } catch (e) {
    // Ignore invalid JSON lines
  }
});

rl.on('close', () => {
  console.log(`Processed ${csvRows.length - 1} data points`);

  // Write CSV
  fs.writeFileSync(csvFile, csvRows.join('\n'));
  console.log(`CSV written to: ${csvFile}`);

  // Generate summary
  const summary = generateSummary();
  fs.writeFileSync(summaryFile, summary);
  console.log(`Summary written to: ${summaryFile}`);

  console.log('\n' + summary);
});

/**
 * Calculate percentile from array
 */
function percentile(arr, p) {
  if (arr.length === 0) return 0;
  const sorted = arr.slice().sort((a, b) => a - b);
  const index = Math.ceil(sorted.length * p) - 1;
  return sorted[Math.max(0, index)];
}

/**
 * Generate summary statistics
 */
function generateSummary() {
  const lines = [];

  lines.push('='.repeat(80));
  lines.push('LOAD TEST RESULTS SUMMARY');
  lines.push('='.repeat(80));
  lines.push('');

  // Overall statistics
  lines.push('OVERALL STATISTICS');
  lines.push('-'.repeat(80));

  if (metrics.http_req_duration.length > 0) {
    lines.push(`Total Requests: ${metrics.http_req_duration.length}`);
    lines.push(`Failed Requests: ${failedRequests}`);
    lines.push(`Success Rate: ${((1 - failedRequests / metrics.http_req_duration.length) * 100).toFixed(2)}%`);
    lines.push('');

    lines.push('HTTP Request Duration:');
    lines.push(`  Min:    ${metrics.http_req_duration.reduce((a, b) => Math.min(a, b), Infinity).toFixed(2)} ms`);
    lines.push(`  Mean:   ${(metrics.http_req_duration.reduce((a, b) => a + b, 0) / metrics.http_req_duration.length).toFixed(2)} ms`);
    lines.push(`  Median: ${percentile(metrics.http_req_duration, 0.5).toFixed(2)} ms`);
    lines.push(`  p90:    ${percentile(metrics.http_req_duration, 0.9).toFixed(2)} ms`);
    lines.push(`  p95:    ${percentile(metrics.http_req_duration, 0.95).toFixed(2)} ms`);
    lines.push(`  p99:    ${percentile(metrics.http_req_duration, 0.99).toFixed(2)} ms`);
    lines.push(`  Max:    ${metrics.http_req_duration.reduce((a, b) => Math.max(a, b), -Infinity).toFixed(2)} ms`);
  }

  lines.push('');
  lines.push('');

  // By Scenario
  lines.push('STATISTICS BY SCENARIO');
  lines.push('-'.repeat(80));

  for (const [scenario, durations] of Object.entries(metricsByScenario)) {
    if (durations.length === 0) continue;

    lines.push(`\nScenario: ${scenario}`);
    lines.push(`  Requests: ${durations.length}`);
    lines.push(`  Mean:   ${(durations.reduce((a, b) => a + b, 0) / durations.length).toFixed(2)} ms`);
    lines.push(`  Median: ${percentile(durations, 0.5).toFixed(2)} ms`);
    lines.push(`  p95:    ${percentile(durations, 0.95).toFixed(2)} ms`);
    lines.push(`  p99:    ${percentile(durations, 0.99).toFixed(2)} ms`);
    lines.push(`  Max:    ${durations.reduce((a, b) => Math.max(a, b), -Infinity).toFixed(2)} ms`);
  }

  lines.push('');
  lines.push('');

  // By Endpoint
  lines.push('STATISTICS BY ENDPOINT TYPE');
  lines.push('-'.repeat(80));

  // Sort endpoints by request count
  const sortedEndpoints = Object.entries(metricsByEndpoint)
    .sort(([, a], [, b]) => b.length - a.length);

  for (const [endpoint, durations] of sortedEndpoints) {
    if (durations.length === 0) continue;

    lines.push(`\nEndpoint: ${endpoint}`);
    lines.push(`  Requests: ${durations.length}`);
    lines.push(`  Mean:   ${(durations.reduce((a, b) => a + b, 0) / durations.length).toFixed(2)} ms`);
    lines.push(`  Median: ${percentile(durations, 0.5).toFixed(2)} ms`);
    lines.push(`  p95:    ${percentile(durations, 0.95).toFixed(2)} ms`);
    lines.push(`  p99:    ${percentile(durations, 0.99).toFixed(2)} ms`);
    lines.push(`  Max:    ${durations.reduce((a, b) => Math.max(a, b), -Infinity).toFixed(2)} ms`);
  }

  lines.push('');
  lines.push('');

  // Errors
  if (errors.length > 0) {
    lines.push('ERRORS');
    lines.push('-'.repeat(80));
    lines.push(`Total Errors: ${errors.length}`);
    lines.push('');

    // Group errors by endpoint
    const errorsByEndpoint = {};
    errors.forEach(err => {
      if (!errorsByEndpoint[err.endpoint]) {
        errorsByEndpoint[err.endpoint] = [];
      }
      errorsByEndpoint[err.endpoint].push(err);
    });

    for (const [endpoint, errs] of Object.entries(errorsByEndpoint)) {
      lines.push(`  ${endpoint}: ${errs.length} errors`);
    }
  }

  lines.push('');
  lines.push('='.repeat(80));

  return lines.join('\n');
}
