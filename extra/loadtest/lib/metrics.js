// Custom k6 metrics for tracking API performance

import { Trend, Counter, Rate } from 'k6/metrics';

// Response size tracking
export const responseSizeTrend = new Trend('response_size_bytes');
export const changesetCountTrend = new Trend('changeset_count');

// Queue depth estimation (based on response time degradation)
export const queueDepthEstimate = new Trend('estimated_queue_depth');

// Memory pressure indicators
export const largeResponseCounter = new Counter('large_responses');  // Responses >1MB
export const hugeResponseCounter = new Counter('huge_responses');    // Responses >10MB

// Error tracking by type
export const timeoutErrors = new Counter('timeout_errors');
export const serverErrors = new Counter('server_errors');
export const clientErrors = new Counter('client_errors');

// Endpoint-specific success rates
export const healthCheckRate = new Rate('health_check_success');
export const singleChangesetRate = new Rate('single_changeset_success');
export const dayQueryRate = new Rate('day_query_success');
export const rangeQueryRate = new Rate('range_query_success');

/**
 * Record response metrics
 * @param {Response} res - k6 HTTP response object
 * @param {string} endpointType - Type of endpoint (health, single, day, range, etc.)
 * @param {number} changesetCount - Optional: number of changesets returned
 */
export function recordResponse(res, endpointType, changesetCount = null) {
  // Track response size
  const responseSize = res.body ? res.body.length : 0;
  responseSizeTrend.add(responseSize);

  // Track large responses (potential memory issues)
  if (responseSize > 1048576) {  // 1MB
    largeResponseCounter.add(1);
  }
  if (responseSize > 10485760) {  // 10MB
    hugeResponseCounter.add(1);
  }

  // Track changeset count if available
  if (changesetCount !== null) {
    changesetCountTrend.add(changesetCount);
  }

  // Estimate queue depth from response time
  // If response time is unusually high, assume queries are queuing
  const duration = res.timings.duration;
  if (duration > 5000) {  // Over 5 seconds suggests queuing
    const estimatedQueueSize = Math.floor(duration / 100);
    queueDepthEstimate.add(estimatedQueueSize);
  }

  // Track errors by type
  if (res.status === 0 || res.error) {
    timeoutErrors.add(1);
  } else if (res.status >= 500) {
    serverErrors.add(1);
  } else if (res.status >= 400 && res.status < 500) {
    clientErrors.add(1);
  }

  // Track success rates by endpoint type
  const success = res.status === 200;
  switch (endpointType) {
    case 'health':
      healthCheckRate.add(success);
      break;
    case 'single':
      singleChangesetRate.add(success);
      break;
    case 'day':
      dayQueryRate.add(success);
      break;
    case 'range':
      rangeQueryRate.add(success);
      break;
  }

  return {
    success,
    responseSize,
    changesetCount,
    duration
  };
}

/**
 * Try to extract changeset count from response
 * @param {Response} res - k6 HTTP response object
 * @param {string} format - Response format (json, xml, jsonl)
 * @returns {number|null}
 */
export function extractChangesetCount(res, format = 'json') {
  if (!res.body || res.status !== 200) {
    return null;
  }

  try {
    if (format === 'json') {
      const data = JSON.parse(res.body);
      if (data.changesets && Array.isArray(data.changesets)) {
        return data.changesets.length;
      }
    } else if (format === 'jsonl') {
      // Count newlines
      return res.body.split('\n').filter(line => line.trim()).length;
    }
    // For XML, we'd need to parse, but that's expensive - skip for now
  } catch (e) {
    // Ignore parse errors
  }

  return null;
}
