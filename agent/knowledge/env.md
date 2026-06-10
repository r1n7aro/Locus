# Environment
OS: <os> (<arch>)
Shell: <shell>
Python: <python>
Working directory: <working_dir>
{{#git}}

## Git Context
Branch: <git_branch>

### Recent Commits
```
<git_recent_commits>
```
{{#git_uncommitted}}

### Uncommitted Changes
```
<git_uncommitted_stat>
```
{{/git_uncommitted}}
{{/git}}
{{#unity}}
Unity Editor: <unity_version>

## Current Unity State
Unity Editor Status: <unity_status>
Allowed Status Values: `disconnected` | `editing` | `playing` | `playing_paused`
Active Scene: <unity_active_scene>

## Project Configuration
Render Pipeline: <render_pipeline>
Input System: <input_system>

### Tags
<custom_tags>
### Layers
<layer_list>
### Physics
<physics_config>
{{/unity}}
{{#knowledge}}

<knowledge_context>
{{/knowledge}}
{{#knowledge_index}}{{/knowledge_index}}
{{#knowledge_memory}}{{/knowledge_memory}}
