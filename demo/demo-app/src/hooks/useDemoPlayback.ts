import { useCallback, useRef, useState } from 'react';
import type { TerminalHandle } from './useTerminal';
import type { PlaybackStep } from '../lib/demo-scenarios';

export interface DemoPlaybackState {
  currentStep: number;
  isPlaying: boolean;
  isPaused: boolean;
  speed: number;
}

export function useDemoPlayback() {
  const [currentStep, setCurrentStep] = useState(-1);
  const [isPlaying, setIsPlaying] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
  const [speed, setSpeed] = useState(1);

  const terminalsRef = useRef<(React.RefObject<TerminalHandle | null>)[]>([]);
  const stepsRef = useRef<PlaybackStep[]>([]);
  const cancelRef = useRef(false);
  const pauseRef = useRef(false);
  const speedRef = useRef(1);
  const playingRef = useRef(false); // ref-based guard (state can lag in batched updates)

  // Keep speedRef in sync
  const updateSpeed = useCallback((s: number) => {
    speedRef.current = s;
    setSpeed(s);
  }, []);

  const setTerminals = useCallback((handles: (React.RefObject<TerminalHandle | null>)[]) => {
    terminalsRef.current = handles;
  }, []);

  const setSteps = useCallback((steps: PlaybackStep[]) => {
    stepsRef.current = steps;
  }, []);

  const sleep = useCallback((ms: number) => {
    return new Promise<void>((resolve) => {
      const adjusted = ms / speedRef.current;
      const start = Date.now();
      const check = () => {
        if (cancelRef.current) { resolve(); return; }
        if (pauseRef.current) { setTimeout(check, 50); return; }
        const elapsed = Date.now() - start;
        if (elapsed >= adjusted) { resolve(); return; }
        setTimeout(check, Math.min(50, adjusted - elapsed));
      };
      check();
    });
  }, []);

  const play = useCallback(async () => {
    if (playingRef.current) return; // ref guard — immune to batched state lag
    playingRef.current = true;
    cancelRef.current = false;
    pauseRef.current = false;
    setIsPlaying(true);
    setIsPaused(false);

    const steps = stepsRef.current;

    for (let i = 0; i < steps.length; i++) {
      if (cancelRef.current) break;

      setCurrentStep(i);
      const step = steps[i];
      const handle = terminalsRef.current[step.terminal]?.current;
      const term = handle?.terminal;

      // Wait before typing
      await sleep(step.delay_before_ms);
      if (cancelRef.current) break;

      if (term) {
        // Write prompt marker
        term.write('\x1b[38;5;139m$ \x1b[0m');

        // Type command character by character directly into xterm
        for (const char of step.command) {
          if (cancelRef.current) break;
          term.write(char);
          await sleep(step.type_speed_ms);
        }
        if (cancelRef.current) break;

        // Newline after command
        term.write('\r\n');

        // Write simulated output line by line
        if (step.output) {
          // Small pause before output starts appearing
          await sleep(Math.min(400, step.wait_after_ms / 3) / speedRef.current);
          if (cancelRef.current) break;

          for (const line of step.output) {
            if (cancelRef.current) break;
            term.write(line + '\r\n');
            // Stagger output lines for realism
            await sleep(60);
          }
        }
      }

      // Wait remaining time after command
      await sleep(step.wait_after_ms * 0.5);
      if (cancelRef.current) break;
    }

    playingRef.current = false;
    setIsPlaying(false);
    setIsPaused(false);
  }, [sleep]);

  const pause = useCallback(() => {
    pauseRef.current = true;
    setIsPaused(true);
  }, []);

  const resume = useCallback(() => {
    pauseRef.current = false;
    setIsPaused(false);
  }, []);

  const reset = useCallback(() => {
    cancelRef.current = true;
    pauseRef.current = false;
    playingRef.current = false;
    setCurrentStep(-1);
    setIsPlaying(false);
    setIsPaused(false);
    // Clear all terminals
    for (const ref of terminalsRef.current) {
      ref.current?.terminal?.clear();
    }
  }, []);

  return {
    play,
    pause,
    resume,
    reset,
    currentStep,
    isPlaying,
    isPaused,
    speed,
    setSpeed: updateSpeed,
    setTerminals,
    setSteps,
  };
}
