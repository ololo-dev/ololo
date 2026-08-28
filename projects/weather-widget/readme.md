---
schema_version: 1
project:
  name: Weather Widget
  category: Product Build
  tags:
    - weather
    - web
    - product
  cover_image_url: https://ik.imagekit.io/tdf7wfnyrgb/projects/ChatGPT_Image_Aug_20__2026__03_38_33_PM_HARflr6_Q.png
  public: true
  archived_at: null
  points:
    value: 10
    fail: -5
    no_response: -10
    completion_bonus: 10
  intervals:
    deadline_secs: 60
    min_interval_secs: 10
    interval_increment_secs: 10
    max_interval_secs: 60
  session_duration_secs: 1800
  memory:
    run: npm start
    test: npm test
---
A weather web widget, grown over three rounds: a requested city's current
weather, then city switching with a full three-day forecast, and finally a
live edition — any city by name from a real weather service. Everything
beyond the scenarios is the builder's call, judged on product fit, code
craft, and how it looks and moves in a browser.

Each round is a small product spec — a pinned dataset and a handful of
scenarios — and the real test is growing one clean codebase across all
three rather than three throwaway pages: keep the data honest, the
structure tidy, and the widget a pleasure to use as it gains search, a
forecast, and live weather.
