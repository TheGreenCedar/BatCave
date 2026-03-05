using BatCave.Layouts;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.Threading;

namespace BatCave;

public sealed partial class MainWindow : Window
{

    private bool _bootstrapped;
    private long _selectionSettleProbeStartedAt;
    private bool _logicalCpuGridLayoutQueued;
    private int _logicalCpuGridLastCount = -1;
    private double _logicalCpuGridLastWidth = -1;
    private double _logicalCpuGridLastHeight = -1;

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        InitializeComponent();
        ViewModel.AttachDispatcherQueue(DispatcherQueue);
        ViewModel.PropertyChanged += ViewModel_PropertyChanged;
        ViewModel.GlobalCpuLogicalProcessorRows.CollectionChanged += GlobalCpuLogicalProcessorRows_CollectionChanged;
        Activated += OnActivated;
        SizeChanged += OnWindowSizeChanged;
        Closed += OnWindowClosed;
    }

    public MonitoringShellViewModel ViewModel { get; }

    private async void OnActivated(object sender, WindowActivatedEventArgs args)
    {
        if (_bootstrapped)
        {
            return;
        }

        _bootstrapped = true;
        await ViewModel.BootstrapAsync(CancellationToken.None);
        SyncAdminToggleState();
        GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
        ScheduleLogicalCpuGridLayout();
    }

    private async void AdminModeToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (sender is not ToggleSwitch toggle)
        {
            return;
        }

        if (toggle.Visibility != Visibility.Visible)
        {
            return;
        }

        if (!CanInteractWithAdminToggle())
        {
            if (toggle.IsOn != ViewModel.AdminModeEnabled)
            {
                toggle.IsOn = ViewModel.AdminModeEnabled;
            }

            return;
        }

        try
        {
            await ViewModel.ToggleAdminModeAsync(toggle.IsOn, CancellationToken.None);
        }
        finally
        {
            SyncAdminToggleState();
        }
    }

    private void FocusFilterAccelerator_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        TextBox target = HeaderControlsPhone.Visibility == Visibility.Visible
            ? FilterTextBoxPhone
            : FilterTextBox;
        target.Focus(FocusState.Programmatic);
        target.SelectAll();
    }

    private async void RetryAccelerator_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (ViewModel.StartupErrorVisibility == Visibility.Visible
            && ViewModel.RetryBootstrapRequestedCommand.CanExecute(null))
        {
            await ViewModel.RetryBootstrapRequestedCommand.ExecuteAsync(null);
        }
    }

    private void ClearSelectionAccelerator_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (ViewModel.ClearSelectionRequestedCommand.CanExecute(null))
        {
            ViewModel.ClearSelectionRequestedCommand.Execute(null);
        }
    }

    private void ViewModel_PropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is null)
        {
            return;
        }

        switch (e.PropertyName)
        {
            case nameof(MonitoringShellViewModel.IsLive):
            case nameof(MonitoringShellViewModel.AdminModePending):
            case nameof(MonitoringShellViewModel.AdminModeEnabled):
                SyncAdminToggleState();
                break;
            case nameof(MonitoringShellViewModel.GlobalCpuLogicalGridVisibility):
                ScheduleLogicalCpuGridLayout();
                break;
            case nameof(MonitoringShellViewModel.SelectedGlobalResource):
                if (!ReferenceEquals(GlobalResourceListView.SelectedItem, ViewModel.SelectedGlobalResource))
                {
                    GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
                }

                break;
        }
    }

    private bool CanInteractWithAdminToggle()
    {
        return ViewModel.IsLive && !ViewModel.AdminModePending;
    }

    private void SyncAdminToggleState()
    {
        bool canInteract = CanInteractWithAdminToggle();
        AdminModeToggle.IsEnabled = canInteract;
        AdminModeTogglePhone.IsEnabled = canInteract;
    }

    private void ProcessListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (sender is not ListView listView)
        {
            return;
        }

        if (listView.SelectedItem is ProcessRowViewState selected)
        {
            BeginSelectionSettleProbeIfNeeded();
            _ = ViewModel.SelectRowAsync(selected.Sample, CancellationToken.None);
            DispatcherQueue.TryEnqueue(CompleteSelectionSettleProbeIfPending);
            return;
        }

        if (ViewModel.SelectedVisibleRowBinding is not null)
        {
            // Ignore transient null churn from virtualization/sort transitions.
            BeginSelectionSettleProbeIfNeeded();
            ViewModel.SelectedVisibleRowBinding = null;
            DispatcherQueue.TryEnqueue(CompleteSelectionSettleProbeIfPending);
            return;
        }

        CompleteSelectionSettleProbeIfPending();
    }

    private void GlobalResourceListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (sender is not ListView listView)
        {
            return;
        }

        try
        {
            GlobalResourceRowViewState? selected = listView.SelectedItem as GlobalResourceRowViewState;
            if (!ReferenceEquals(ViewModel.SelectedGlobalResource, selected))
            {
                ViewModel.SelectedGlobalResource = selected;
            }
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[GlobalSelection] Failed to apply selection from list view. {ex}");
            ViewModel.SelectedGlobalResource = null;
        }
    }

    private void OnWindowSizeChanged(object sender, WindowSizeChangedEventArgs args)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalGridView_Loaded(object sender, RoutedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalGridView_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalProcessorRows_CollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void ScheduleLogicalCpuGridLayout()
    {
        if (_logicalCpuGridLayoutQueued)
        {
            return;
        }

        _logicalCpuGridLayoutQueued = true;
        DispatcherQueue.TryEnqueue(() =>
        {
            _logicalCpuGridLayoutQueued = false;
            ApplyLogicalCpuGridLayout();
        });
    }

    private void ApplyLogicalCpuGridLayout()
    {
        if (GlobalCpuLogicalGridView.Visibility != Visibility.Visible)
        {
            return;
        }

        if (GlobalCpuLogicalGridView.ItemsPanelRoot is not ItemsWrapGrid itemsWrapGrid)
        {
            return;
        }

        int logicalProcessorCount = ViewModel.GlobalCpuLogicalProcessorRows.Count;
        if (logicalProcessorCount <= 0)
        {
            return;
        }

        double availableWidth = Math.Max(0, GlobalCpuLogicalGridView.ActualWidth - 4);
        double availableHeight = Math.Max(0, GlobalCpuLogicalGridView.ActualHeight - 4);
        if (availableWidth < 1 || availableHeight < 1)
        {
            return;
        }

        if (_logicalCpuGridLastCount == logicalProcessorCount &&
            Math.Abs(_logicalCpuGridLastWidth - availableWidth) < 0.5 &&
            Math.Abs(_logicalCpuGridLastHeight - availableHeight) < 0.5)
        {
            return;
        }

        LogicalCpuGridLayoutResult layout = LogicalCpuGridLayout.Resolve(
            logicalProcessorCount,
            availableWidth,
            availableHeight);

        itemsWrapGrid.MaximumRowsOrColumns = layout.Columns;
        itemsWrapGrid.ItemWidth = layout.ItemWidth;
        itemsWrapGrid.ItemHeight = layout.ItemHeight;

        _logicalCpuGridLastCount = logicalProcessorCount;
        _logicalCpuGridLastWidth = availableWidth;
        _logicalCpuGridLastHeight = availableHeight;
    }


    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        ViewModel.PropertyChanged -= ViewModel_PropertyChanged;
        ViewModel.GlobalCpuLogicalProcessorRows.CollectionChanged -= GlobalCpuLogicalProcessorRows_CollectionChanged;
        SizeChanged -= OnWindowSizeChanged;
        Closed -= OnWindowClosed;
    }

    private void BeginSelectionSettleProbeIfNeeded()
    {
        if (_selectionSettleProbeStartedAt <= 0)
        {
            _selectionSettleProbeStartedAt = Stopwatch.GetTimestamp();
        }
    }

    private void CompleteSelectionSettleProbeIfPending()
    {
        if (_selectionSettleProbeStartedAt <= 0)
        {
            return;
        }

        long startedAt = _selectionSettleProbeStartedAt;
        _selectionSettleProbeStartedAt = 0;
        ViewModel.RecordSelectionSettleProbe(Stopwatch.GetTimestamp() - startedAt);
    }
}

