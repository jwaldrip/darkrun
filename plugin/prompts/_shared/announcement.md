{# Standard run/station banner shared by every per-action template. #}
> **Run** `{{ run }}`{% if station %} · **Station** `{{ station }}`{% endif %}{% if phase %} · **Phase** `{{ phase }}`{% endif %}
{% if kills %}
> Eliminates: _{{ kills }}_
{% endif %}
