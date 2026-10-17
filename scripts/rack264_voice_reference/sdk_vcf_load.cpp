// SPDX-License-Identifier: MIT
// Original loading audit, using the genuine SDK loader and the unchanged pinned
// Fundamental 2.6.4 VCF prefix in reference_module.hpp. No replacement loader.
#include "reference_module.hpp"
#include <context.hpp>
#include <engine/Engine.hpp>
#include <plugin.hpp>
#include <algorithm>
#include <array>
#include <cstdio>
#include <cstdlib>
#include <exception>
#include <initializer_list>
#include <memory>
#include <stdexcept>
#include <string>
#include <utility>

using Parameters = std::array<float, 6>;
static const int paramIds[] = {VCF::FREQ_PARAM, VCF::RES_PARAM, VCF::FREQ_CV_PARAM,
                               VCF::RES_CV_PARAM, VCF::DRIVE_PARAM, VCF::DRIVE_CV_PARAM};
static const char* paramNames[] = {"frequency", "resonance", "cutoff_cv_amount",
                                  "resonance_cv_amount", "drive", "drive_cv_amount"};
static const Parameters defaults = {0.5f, 0.f, 0.f, 0.f, 0.f, 0.f};

struct RestoreContext {
    rack::Context* previous = rack::contextGet();
    ~RestoreContext() { rack::contextSet(previous); }
};

struct JsonDeleter {
    void operator()(json_t* value) const { json_decref(value); }
};
using Json = std::unique_ptr<json_t, JsonDeleter>;

// Register actual SDK Plugin/Model objects, so modelFromJson()/fromJson() use
// their real registry lookup. Plugin owns its Model; modules die before it does.
struct Registry {
    rack::plugin::Plugin plugin;
    rack::plugin::Model* model;
    Registry() : model(new rack::plugin::Model) {
        plugin.slug = "Fundamental";
        plugin.version = "2.6.4";
        plugin.name = "Fundamental";
        model->slug = "VCF";
        model->name = "VCF";
        plugin.addModel(model);
        rack::plugin::plugins.push_back(&plugin);
    }
    ~Registry() {
        auto& plugins = rack::plugin::plugins;
        plugins.erase(std::remove(plugins.begin(), plugins.end(), &plugin), plugins.end());
    }
    std::unique_ptr<VCF> create() const {
        auto module = std::make_unique<VCF>();
        module->model = model;
        return module;
    }
};

static Parameters parameters(VCF& module) {
    Parameters values{};
    for (int i = 0; i < 6; ++i) values[i] = module.params[paramIds[i]].getValue();
    return values;
}

static json_t* parametersJson(const Parameters& values) {
    json_t* result = json_object();
    for (int i = 0; i < 6; ++i) json_object_set_new(result, paramNames[i], json_real(values[i]));
    return result;
}

static Json savedModule(bool hasParams, std::initializer_list<std::pair<int, float>> entries = {}) {
    Json root(json_object());
    json_object_set_new(root.get(), "plugin", json_string("Fundamental"));
    json_object_set_new(root.get(), "model", json_string("VCF"));
    json_object_set_new(root.get(), "version", json_string("2.6.4"));
    if (hasParams) {
        json_t* params = json_array();
        for (const auto& entry : entries) {
            json_t* parameter = json_object();
            json_object_set_new(parameter, "id", json_integer(entry.first));
            json_object_set_new(parameter, "value", json_real(entry.second));
            json_array_append_new(params, parameter);
        }
        json_object_set_new(root.get(), "params", params);
    }
    return root;
}

static void load(VCF& module, json_t* saved) {
    if (rack::plugin::modelFromJson(saved) != module.model)
        throw std::runtime_error("genuine registry did not resolve the registered VCF model");
    // Actual SDK Module::fromJson dispatches the source VCF::paramsFromJson.
    module.fromJson(saved);
}

static bool result(json_t* cases, const char* name, const Parameters& actual,
                   const Parameters& expected, json_t** resultOut = nullptr) {
    const bool passed = actual == expected;
    json_t* entry = json_object();
    json_object_set_new(entry, "name", json_string(name));
    json_object_set_new(entry, "actual", parametersJson(actual));
    json_object_set_new(entry, "expected", parametersJson(expected));
    json_object_set_new(entry, "passed", json_boolean(passed));
    json_array_append_new(cases, entry);
    if (resultOut) *resultOut = entry;
    return passed;
}

int main() {
    Json report(json_object());
    json_object_set_new(report.get(), "sdk_version", json_string(rack::APP_VERSION.c_str()));
    json_object_set_new(report.get(), "plugin", json_string("Fundamental"));
    json_object_set_new(report.get(), "plugin_version", json_string("2.6.4"));
    json_object_set_new(report.get(), "model", json_string("VCF"));
    json_object_set_new(report.get(), "loader", json_string("genuine Module::fromJson"));
    json_t* cases = json_array();
    json_object_set_new(report.get(), "cases", cases);
    bool allPassed = rack::APP_VERSION == "2.6.6";
    try {
        // Genuine ParamQuantity::setImmediateValue/reset call APP->engine.
        // Engine construction records contextGet(), so install Context first.
        // Context owns/deletes Engine. The restore guard outlives Context and
        // keeps APP valid through its destructor. No engine threads are started,
        // and no audio, driver, window, scene, or patch manager is initialized.
        RestoreContext restore;
        rack::Context context;
        rack::contextSet(&context);
        context.engine = new rack::engine::Engine;
        json_object_set_new(report.get(), "context", json_string("genuine headless Context and Engine"));
        Registry registry;
        auto constructor = registry.create();
        allPassed &= result(cases, "constructor", parameters(*constructor), defaults);
        auto run = [&](const char* name, Json saved, const Parameters& expected) {
            auto module = registry.create();
            load(*module, saved.get());
            allPassed &= result(cases, name, parameters(*module), expected);
        };
        run("missing_params", savedModule(false), defaults);
        run("empty_params", savedModule(true), {0.5f, 0.f, 0.f, 1.f, 0.f, 1.f});
        run("partial_params", savedModule(true, {{VCF::FREQ_PARAM, 0.625f}}),
            {0.625f, 0.f, 0.f, 1.f, 0.f, 1.f});
        run("explicit_zero_depths", savedModule(true, {{VCF::RES_CV_PARAM, 0.f},
            {VCF::DRIVE_CV_PARAM, 0.f}}), defaults);
        run("explicit_nonzero_depths", savedModule(true, {{VCF::RES_CV_PARAM, -0.5f},
            {VCF::DRIVE_CV_PARAM, 0.75f}}), {0.5f, 0.f, 0.f, -0.5f, 0.f, 0.75f});
        run("omitted_resonance_depth", savedModule(true, {{VCF::DRIVE_CV_PARAM, 0.25f}}),
            {0.5f, 0.f, 0.f, 1.f, 0.f, 0.25f});
        run("omitted_drive_depth", savedModule(true, {{VCF::RES_CV_PARAM, -0.5f}}),
            {0.5f, 0.f, 0.f, -0.5f, 0.f, 1.f});

        auto reset = registry.create();
        const Parameters edited = {0.625f, 0.75f, -0.5f, 0.25f, 0.5f, -0.75f};
        for (int i = 0; i < 6; ++i) reset->params[paramIds[i]].setValue(edited[i]);
        // Explicit controlled histories avoid reading the source constructor's
        // undefined previous-input value. They are never processed as audio.
        for (auto& filter : reset->filters) {
            filter.input = rack::simd::float_4(0.625f);
            for (int i = 0; i < 4; ++i) filter.state[i] = rack::simd::float_4(0.25f * (i + 1));
        }
        // Use the actual SDK event overload through Module. It resets parameter
        // quantities, then dispatches the pinned VCF's deprecated onReset().
        rack::engine::Module* resetBase = reset.get();
        resetBase->onReset(rack::engine::Module::ResetEvent{});
        json_t* resetResult = nullptr;
        allPassed &= result(cases, "reset_before_load", parameters(*reset), defaults, &resetResult);
        bool statesCleared = true;
        bool inputsPreserved = true;
        for (const auto& filter : reset->filters) {
            for (int lane = 0; lane < 4; ++lane) {
                inputsPreserved &= filter.input[lane] == 0.625f;
                for (int i = 0; i < 4; ++i) statesCleared &= filter.state[i][lane] == 0.f;
            }
        }
        json_object_set_new(resetResult, "filter_states_cleared", json_boolean(statesCleared));
        json_object_set_new(resetResult, "previous_inputs_preserved", json_boolean(inputsPreserved));
        const bool resetPassed = parameters(*reset) == defaults && statesCleared && inputsPreserved;
        json_object_set_new(resetResult, "passed", json_boolean(resetPassed));
        allPassed &= resetPassed;
        auto partial = savedModule(true, {{VCF::FREQ_PARAM, 0.625f}});
        load(*reset, partial.get());
        allPassed &= result(cases, "reset_then_partial_load", parameters(*reset),
            {0.625f, 0.f, 0.f, 1.f, 0.f, 1.f});
    } catch (const std::exception& error) {
        allPassed = false;
        json_object_set_new(report.get(), "error", json_string(error.what()));
    }
    json_object_set_new(report.get(), "all_passed", json_boolean(allPassed));
    char* serialized = json_dumps(report.get(), JSON_COMPACT | JSON_SORT_KEYS);
    if (!serialized) return 2;
    std::puts(serialized);
    std::free(serialized);
    return allPassed ? 0 : 1;
}
