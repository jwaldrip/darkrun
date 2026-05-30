{# Renders the station's named agents. Reused by spec/manufacture/review/audit. #}
{% if explorers %}
**Explorers** ({{ explorers | length }}): {% for e in explorers %}`{{ e }}`{% if not loop.last %}, {% endif %}{% endfor %}
{% endif %}
{% if workers %}
**Workers** ({{ workers | length }}): {% for w in workers %}`{{ w }}`{% if not loop.last %} → {% endif %}{% endfor %}
{% endif %}
{% if reviewers %}
**Reviewers** ({{ reviewers | length }}): {% for r in reviewers %}`{{ r }}`{% if not loop.last %}, {% endif %}{% endfor %}
{% endif %}
