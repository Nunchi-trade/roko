export interface TimelineStepState {
  label: string;
  sublabel?: string;
  status: 'pending' | 'active' | 'completed';
}

export class PlaybackController {
  mode: 'auto' | 'step' = 'auto';
  private _stepResolve: (() => void) | null = null;
  private _currentStep = 0;
  private _totalSteps = 0;
  private _onProgress?: (step: number, total: number, cmd: string) => void;

  onProgress(fn: (step: number, total: number, cmd: string) => void) {
    this._onProgress = fn;
  }

  async waitForStep(): Promise<void> {
    if (this.mode === 'auto') return;
    return new Promise(resolve => {
      this._stepResolve = resolve;
    });
  }

  advanceStep() {
    if (this._stepResolve) {
      const r = this._stepResolve;
      this._stepResolve = null;
      r();
    }
  }

  setProgress(n: number, total: number, cmd: string) {
    this._currentStep = n;
    this._totalSteps = total;
    this._onProgress?.(n, total, cmd);
  }

  setMode(mode: 'auto' | 'step') {
    this.mode = mode;
    if (mode === 'auto' && this._stepResolve) this.advanceStep();
  }

  get currentStep() {
    return this._currentStep;
  }

  get totalSteps() {
    return this._totalSteps;
  }

  reset() {
    this._stepResolve = null;
    this._currentStep = 0;
    this._totalSteps = 0;
  }
}

export class TimelineStepper {
  steps: { label: string; sublabel?: string }[] = [];
  activeIndex = -1;
  private _onChange?: (steps: TimelineStepState[]) => void;

  onChange(fn: (steps: TimelineStepState[]) => void) {
    this._onChange = fn;
  }

  init(stepDefs: { label: string; sublabel?: string }[]) {
    this.steps = stepDefs;
    this.activeIndex = -1;
    this._notify();
  }

  setActive(idx: number) {
    this.activeIndex = idx;
    this._notify();
  }

  reset() {
    this.activeIndex = -1;
    this._notify();
  }

  private _notify() {
    this._onChange?.(
      this.steps.map((s, i) => ({
        ...s,
        status:
          i < this.activeIndex
            ? ('completed' as const)
            : i === this.activeIndex
              ? ('active' as const)
              : ('pending' as const),
      })),
    );
  }
}
