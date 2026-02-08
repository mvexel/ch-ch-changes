// Utility functions for load testing

import { DATE_RANGES, CHANGESET_ID_MAX, CHANGESET_ID_MIN, KNOWN_USERS } from './config.js';

/**
 * Get a random date from a specified period
 * @param {string} period - 'RECENT', 'MID', or 'EARLY'
 * @returns {Date}
 */
export function getRandomDate(period = 'RECENT') {
  const range = DATE_RANGES[period];
  if (!range) {
    throw new Error(`Invalid period: ${period}. Use 'RECENT', 'MID', or 'EARLY'`);
  }

  const startTime = range.start.getTime();
  const endTime = range.end.getTime();
  const randomTime = startTime + Math.random() * (endTime - startTime);

  return new Date(randomTime);
}

/**
 * Get a random date based on weighted distribution
 * 60% recent, 30% mid, 10% early
 * @returns {Date}
 */
export function getWeightedRandomDate() {
  const r = Math.random();
  if (r < 0.6) {
    return getRandomDate('RECENT');
  } else if (r < 0.9) {
    return getRandomDate('MID');
  } else {
    return getRandomDate('EARLY');
  }
}

/**
 * Get a random date range
 * @param {number} maxDays - Maximum number of days in the range
 * @param {string} period - Optional period constraint
 * @returns {{start: Date, end: Date}}
 */
export function getRandomDateRange(maxDays = 7, period = null) {
  const startDate = period ? getRandomDate(period) : getWeightedRandomDate();
  const endDate = new Date(startDate);

  // Random range between 1 and maxDays
  const daysToAdd = 1 + Math.floor(Math.random() * (maxDays - 1));
  endDate.setDate(endDate.getDate() + daysToAdd);

  return { start: startDate, end: endDate };
}

/**
 * Convert Date to YYYYMMDD string
 * @param {Date} date
 * @returns {string}
 */
export function dateToYYYYMMDD(date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}${month}${day}`;
}

/**
 * Get a random changeset ID
 * Weighted towards existing IDs (80% chance of real ID, 20% chance of non-existent)
 * @returns {number}
 */
export function getRandomChangesetId() {
  const r = Math.random();

  if (r < 0.8) {
    // 80% chance of an ID that likely exists
    // Recent IDs are more common, so weight towards higher numbers
    if (Math.random() < 0.7) {
      // 70% chance of recent ID (last 50M)
      return Math.floor((CHANGESET_ID_MAX - 50000000) + Math.random() * 50000000);
    } else {
      // 30% chance of older ID
      return Math.floor(CHANGESET_ID_MIN + Math.random() * (CHANGESET_ID_MAX - 50000000));
    }
  } else {
    // 20% chance of non-existent ID (for 404 testing)
    return Math.floor(CHANGESET_ID_MAX + Math.random() * 10000000);
  }
}

/**
 * Get a random changeset ID range
 * @param {string} size - 'small' (1k), 'medium' (10k), or 'large' (100k)
 * @returns {{start: number, end: number}}
 */
export function getRandomIdRange(size = 'small') {
  const ranges = {
    small: 1000,
    medium: 10000,
    large: 100000
  };

  const rangeSize = ranges[size] || ranges.small;
  const startId = Math.floor(CHANGESET_ID_MIN + Math.random() * (CHANGESET_ID_MAX - rangeSize));

  return {
    start: startId,
    end: startId + rangeSize
  };
}

/**
 * Get a random weighted ID range size
 * 50% small, 30% medium, 20% large
 * @returns {string}
 */
export function getWeightedIdRangeSize() {
  const r = Math.random();
  if (r < 0.5) return 'small';
  if (r < 0.8) return 'medium';
  return 'large';
}

/**
 * Get a random user for testing user queries
 * 30% top users, 50% random known users, 20% non-existent
 * @returns {string}
 */
export function getRandomUser() {
  const r = Math.random();

  if (r < 0.3) {
    // Top 3 most active users
    return KNOWN_USERS[Math.floor(Math.random() * 3)];
  } else if (r < 0.8) {
    // Any known user
    return KNOWN_USERS[Math.floor(Math.random() * KNOWN_USERS.length)];
  } else {
    // Non-existent user (for 404 testing)
    return `nonexistent_user_${Math.floor(Math.random() * 10000)}`;
  }
}

/**
 * Get a random hour (0-23)
 * @returns {number}
 */
export function getRandomHour() {
  return Math.floor(Math.random() * 24);
}

/**
 * Get a random minute (0-59)
 * @returns {number}
 */
export function getRandomMinute() {
  return Math.floor(Math.random() * 60);
}

/**
 * Format duration in milliseconds to human-readable string
 * @param {number} ms
 * @returns {string}
 */
export function formatDuration(ms) {
  if (ms < 1000) {
    return `${ms}ms`;
  } else if (ms < 60000) {
    return `${(ms / 1000).toFixed(2)}s`;
  } else {
    return `${(ms / 60000).toFixed(2)}m`;
  }
}

/**
 * Calculate percentile from sorted array
 * @param {number[]} sortedArray
 * @param {number} percentile - 0-1 (e.g., 0.95 for p95)
 * @returns {number}
 */
export function calculatePercentile(sortedArray, percentile) {
  if (sortedArray.length === 0) return 0;

  const index = Math.ceil(sortedArray.length * percentile) - 1;
  return sortedArray[Math.max(0, index)];
}
