/**
 * Custom shader materials for phosphor glow effects.
 */
import * as THREE from 'three';

/**
 * Phosphor glow material — soft smoothstep falloff with pow intensity curve.
 * Used for point sprites and halo effects. Additive blending for layered glow.
 */
export function makePhosphorMaterial(color: number, opacity = 0.9): THREE.ShaderMaterial {
  return new THREE.ShaderMaterial({
    uniforms: {
      time:    { value: 0 },
      uColor:  { value: new THREE.Color(color) },
      uOpacity: { value: opacity },
    },
    vertexShader: /* glsl */ `
      attribute float size;
      varying vec3 vColor;
      uniform float time;

      void main() {
        vColor = vec3(1.0);
        vec4 mvPosition = modelViewMatrix * vec4(position, 1.0);
        float pulse = 1.0 + 0.2 * sin(time * 0.6 + position.x * 0.3);
        gl_PointSize = (size > 0.0 ? size : 4.0) * pulse * (300.0 / -mvPosition.z);
        gl_Position = projectionMatrix * mvPosition;
      }
    `,
    fragmentShader: /* glsl */ `
      uniform vec3 uColor;
      uniform float uOpacity;

      void main() {
        vec2 uv = gl_PointCoord - 0.5;
        float d = length(uv);
        if (d > 0.5) discard;
        float a = smoothstep(0.5, 0.0, d);
        a = pow(a, 2.0);
        gl_FragColor = vec4(uColor, a * uOpacity);
      }
    `,
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
}

/**
 * Radial breathing halo material for core objects.
 * Pulses via time uniform update in animation loop.
 */
export function makeHaloMaterial(color: number, intensity = 0.6): THREE.ShaderMaterial {
  return new THREE.ShaderMaterial({
    uniforms: {
      time:       { value: 0 },
      uColor:     { value: new THREE.Color(color) },
      uIntensity: { value: intensity },
    },
    vertexShader: /* glsl */ `
      varying vec3 vNormal;
      varying vec3 vPosition;

      void main() {
        vNormal = normalize(normalMatrix * normal);
        vPosition = (modelViewMatrix * vec4(position, 1.0)).xyz;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */ `
      uniform float time;
      uniform vec3 uColor;
      uniform float uIntensity;
      varying vec3 vNormal;
      varying vec3 vPosition;

      void main() {
        float rim = 1.0 - abs(dot(normalize(-vPosition), vNormal));
        rim = pow(rim, 2.5);
        float breath = 0.8 + 0.2 * sin(time * 1.5);
        float alpha = rim * uIntensity * breath;
        gl_FragColor = vec4(uColor, alpha);
      }
    `,
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    side: THREE.FrontSide,
  });
}
