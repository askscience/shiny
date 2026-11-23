// Generated from crates/shiny-plugin-sdk/src/odp_spec.rs — do not edit.
// Regenerate: UPDATE_SLIDE_SPEC=1 cargo test -p shiny-plugin-sdk odp_spec
export default {
  "bullet": {
    "char": "●",
    "indent": 24.0,
    "relative": 0.7
  },
  "fonts": {
    "display": "Inter Display",
    "fallback": "DejaVu Sans",
    "text": "Inter"
  },
  "layoutNames": [
    "title",
    "section",
    "content",
    "two-column",
    "quote",
    "blank"
  ],
  "layouts": {
    "blank": {
      "background": {
        "color": "light",
        "kind": "solid"
      },
      "dark": false,
      "rules": [],
      "texts": [
        {
          "center": true,
          "color": "body",
          "display": false,
          "field": "body",
          "gap": 0.0,
          "h": 288.0,
          "line": 1.35,
          "list": false,
          "shrink": true,
          "size": 24.0,
          "tracking": 0.0,
          "valign": "middle",
          "w": 536.0,
          "weight": 400,
          "x": 52.0,
          "y": 36.0
        }
      ]
    },
    "content": {
      "background": {
        "color": "light",
        "kind": "solid"
      },
      "dark": false,
      "rules": [
        {
          "color": "accent",
          "h": 5.0,
          "radius": 2.5,
          "w": 46.0,
          "with": "title",
          "x": 52.0,
          "y": 96.0
        }
      ],
      "texts": [
        {
          "center": false,
          "color": "ink",
          "display": true,
          "field": "title",
          "gap": 0.0,
          "h": 70.0,
          "line": 1.1,
          "list": false,
          "shrink": false,
          "size": 32.0,
          "tracking": -0.01,
          "valign": "bottom",
          "w": 536.0,
          "weight": 700,
          "x": 52.0,
          "y": 16.0
        },
        {
          "center": false,
          "color": "body",
          "display": false,
          "field": "bullets",
          "gap": 9.0,
          "h": 212.0,
          "line": 1.35,
          "list": true,
          "shrink": true,
          "size": 19.0,
          "tracking": 0.0,
          "valign": "top",
          "w": 536.0,
          "weight": 400,
          "x": 52.0,
          "y": 120.0
        }
      ]
    },
    "quote": {
      "background": {
        "angle": 150.0,
        "from": "darkA",
        "kind": "gradient",
        "to": "darkB"
      },
      "dark": true,
      "rules": [],
      "texts": [
        {
          "center": false,
          "color": "accent",
          "display": true,
          "field": "",
          "gap": 0.0,
          "h": 60.0,
          "line": 1.0,
          "list": false,
          "literal": "“",
          "shrink": false,
          "size": 54.0,
          "tracking": 0.0,
          "valign": "bottom",
          "w": 80.0,
          "weight": 600,
          "with": "body",
          "x": 64.0,
          "y": 52.0
        },
        {
          "center": false,
          "color": "onDark",
          "display": true,
          "field": "body",
          "gap": 0.0,
          "h": 150.0,
          "line": 1.4,
          "list": false,
          "shrink": true,
          "size": 30.0,
          "tracking": -0.01,
          "valign": "top",
          "w": 512.0,
          "weight": 600,
          "x": 64.0,
          "y": 118.0
        },
        {
          "center": false,
          "color": "attribOnDark",
          "display": false,
          "field": "attribution",
          "gap": 0.0,
          "h": 40.0,
          "line": 1.3,
          "list": false,
          "shrink": false,
          "size": 16.0,
          "tracking": 0.0,
          "valign": "top",
          "w": 512.0,
          "weight": 400,
          "x": 64.0,
          "y": 280.0
        }
      ]
    },
    "section": {
      "background": {
        "angle": 140.0,
        "from": "accent",
        "kind": "gradient",
        "to": "sectionB"
      },
      "dark": true,
      "rules": [],
      "texts": [
        {
          "center": false,
          "color": "onDark",
          "display": true,
          "field": "title",
          "gap": 0.0,
          "h": 240.0,
          "line": 1.05,
          "list": false,
          "shrink": true,
          "size": 54.0,
          "tracking": -0.02,
          "valign": "middle",
          "w": 512.0,
          "weight": 800,
          "x": 64.0,
          "y": 60.0
        }
      ]
    },
    "title": {
      "background": {
        "angle": 150.0,
        "from": "darkA",
        "kind": "gradient",
        "to": "darkB"
      },
      "dark": true,
      "rules": [
        {
          "color": "accent",
          "h": 6.0,
          "radius": 3.0,
          "w": 64.0,
          "with": "title",
          "x": 64.0,
          "y": 202.0
        }
      ],
      "texts": [
        {
          "center": false,
          "color": "onDark",
          "display": true,
          "field": "title",
          "gap": 0.0,
          "h": 150.0,
          "line": 1.05,
          "list": false,
          "shrink": false,
          "size": 46.0,
          "tracking": -0.02,
          "valign": "bottom",
          "w": 512.0,
          "weight": 800,
          "x": 64.0,
          "y": 40.0
        },
        {
          "center": false,
          "color": "subOnDark",
          "display": false,
          "field": "subtitle",
          "gap": 0.0,
          "h": 96.0,
          "line": 1.3,
          "list": false,
          "shrink": true,
          "size": 22.0,
          "tracking": 0.0,
          "valign": "top",
          "w": 512.0,
          "weight": 500,
          "x": 64.0,
          "y": 226.0
        }
      ]
    },
    "two-column": {
      "background": {
        "color": "light",
        "kind": "solid"
      },
      "dark": false,
      "rules": [
        {
          "color": "accent",
          "h": 5.0,
          "radius": 2.5,
          "w": 46.0,
          "with": "title",
          "x": 52.0,
          "y": 96.0
        }
      ],
      "texts": [
        {
          "center": false,
          "color": "ink",
          "display": true,
          "field": "title",
          "gap": 0.0,
          "h": 70.0,
          "line": 1.1,
          "list": false,
          "shrink": false,
          "size": 32.0,
          "tracking": -0.01,
          "valign": "bottom",
          "w": 536.0,
          "weight": 700,
          "x": 52.0,
          "y": 16.0
        },
        {
          "center": false,
          "color": "body",
          "display": false,
          "field": "columns.0",
          "gap": 9.0,
          "h": 212.0,
          "line": 1.35,
          "list": true,
          "shrink": true,
          "size": 19.0,
          "tracking": 0.0,
          "valign": "top",
          "w": 250.0,
          "weight": 400,
          "x": 52.0,
          "y": 120.0
        },
        {
          "center": false,
          "color": "body",
          "display": false,
          "field": "columns.1",
          "gap": 9.0,
          "h": 212.0,
          "line": 1.35,
          "list": true,
          "shrink": true,
          "size": 19.0,
          "tracking": 0.0,
          "valign": "top",
          "w": 250.0,
          "weight": 400,
          "x": 338.0,
          "y": 120.0
        }
      ]
    }
  },
  "minFit": 0.4,
  "page": {
    "h": 360.0,
    "w": 640.0
  },
  "reveals": [
    "all",
    "bullets"
  ],
  "themeNames": [
    "aurora",
    "slate",
    "ocean",
    "mono",
    "ember"
  ],
  "themes": {
    "aurora": {
      "accent": "#6366f1",
      "attribOnDark": "#a7a7b3",
      "body": "#334155",
      "darkA": "#1a1a2e",
      "darkB": "#2b2b52",
      "ink": "#0f172a",
      "light": "#ffffff",
      "onDark": "#ffffff",
      "sectionB": "#363885",
      "subOnDark": "#c1c1ca"
    },
    "ember": {
      "accent": "#f97316",
      "attribOnDark": "#a8a4a2",
      "body": "#334155",
      "darkA": "#1c1917",
      "darkB": "#2d1f14",
      "ink": "#0f172a",
      "light": "#fff7ed",
      "onDark": "#ffffff",
      "sectionB": "#893f0c",
      "subOnDark": "#c2bfbe"
    },
    "mono": {
      "accent": "#18181b",
      "attribOnDark": "#a2a2a2",
      "body": "#334155",
      "darkA": "#111111",
      "darkB": "#1c1c1c",
      "ink": "#0f172a",
      "light": "#ffffff",
      "onDark": "#ffffff",
      "sectionB": "#0d0d0f",
      "subOnDark": "#bebebe"
    },
    "ocean": {
      "accent": "#0ea5e9",
      "attribOnDark": "#9db1be",
      "body": "#334155",
      "darkA": "#082f49",
      "darkB": "#0c4a6e",
      "ink": "#0f172a",
      "light": "#f0f9ff",
      "onDark": "#ffffff",
      "sectionB": "#085b80",
      "subOnDark": "#bac9d1"
    },
    "slate": {
      "accent": "#475569",
      "attribOnDark": "#a2a6ad",
      "body": "#334155",
      "darkA": "#0f172a",
      "darkB": "#1e293b",
      "ink": "#0f172a",
      "light": "#f8fafc",
      "onDark": "#ffffff",
      "sectionB": "#272f3a",
      "subOnDark": "#bec1c6"
    }
  },
  "transitions": [
    "none",
    "fade",
    "slide",
    "push",
    "zoom"
  ]
};
