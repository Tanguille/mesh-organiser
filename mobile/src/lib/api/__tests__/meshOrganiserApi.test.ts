import { describe, it, expect } from 'vitest';
import { meshOrganiserApi } from '../meshOrganiserApi';

describe('MeshOrganiserApi', () => {
  it('should be defined', () => {
    expect(meshOrganiserApi).toBeDefined();
  });

  it('should have getModels function', () => {
    expect(typeof meshOrganiserApi.getModels).toBe('function');
  });

  it('should have getModel function', () => {
    expect(typeof meshOrganiserApi.getModel).toBe('function');
  });

  it('should have importModel function', () => {
    expect(typeof meshOrganiserApi.importModel).toBe('function');
  });

  it('should have sliceModel function', () => {
    expect(typeof meshOrganiserApi.sliceModel).toBe('function');
  });

  it('should have getPrinters function', () => {
    expect(typeof meshOrganiserApi.getPrinters).toBe('function');
  });

  it('should have startPrint function', () => {
    expect(typeof meshOrganiserApi.startPrint).toBe('function');
  });

  it('should have getPrintStatus function', () => {
    expect(typeof meshOrganiserApi.getPrintStatus).toBe('function');
  });

  it('should have getPrintJobs function', () => {
    expect(typeof meshOrganiserApi.getPrintJobs).toBe('function');
  });
});