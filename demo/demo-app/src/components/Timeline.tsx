import './Timeline.css';

interface Step {
  label: string;
  status: 'done' | 'active' | 'pending';
  detail?: string;
}

interface TimelineProps {
  steps: Step[];
}

export default function Timeline({ steps }: TimelineProps) {
  return (
    <div className="timeline">
      {steps.map((step, i) => (
        <div key={i} className={`timeline-step timeline-${step.status}`}>
          <div className="timeline-marker" />
          <div className="timeline-content">
            <div className="timeline-label">{step.label}</div>
            {step.detail && <div className="timeline-detail">{step.detail}</div>}
          </div>
        </div>
      ))}
    </div>
  );
}
