import { describe, it, expect } from 'vitest';

describe('Sample Test Suite', () => {
  it('should pass a basic assertion', () => {
    expect(1 + 1).toBe(2);
  });

  it('should verify string operations', () => {
    const greeting = 'Hello, World!';
    expect(greeting).toContain('World');
    expect(greeting).toHaveLength(13);
  });

  it('should verify array operations', () => {
    const numbers = [1, 2, 3, 4, 5];
    expect(numbers).toHaveLength(5);
    expect(numbers).toContain(3);
  });

  it('should verify object properties', () => {
    const user = {
      name: 'Test User',
      email: 'test@example.com',
    };
    expect(user).toHaveProperty('name');
    expect(user.email).toBe('test@example.com');
  });
});
